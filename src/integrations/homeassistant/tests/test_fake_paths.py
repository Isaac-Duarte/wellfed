"""Paths the simulator cannot reach from Python, on a scripted bridge: a
feed whose answer was lost opens a repair that its settlement clears, a
revoked key or a silent feeder at setup, firmware installs, diagnostics
and stale devices."""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Iterator
from pathlib import Path
from types import SimpleNamespace
from typing import Any

from homeassistant.config_entries import ConfigEntryState
from homeassistant.const import ATTR_ENTITY_ID, STATE_OFF, STATE_ON
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import device_registry as dr, issue_registry as ir
import pytest
from pytest_homeassistant_custom_component.common import MockConfigEntry
from pytest_homeassistant_custom_component.components.diagnostics import (
    get_diagnostics_for_config_entry,
)
from pytest_homeassistant_custom_component.typing import ClientSessionGenerator

from custom_components.wellfed import async_remove_config_entry_device
from custom_components.wellfed.const import (
    CONF_ADDRESS,
    CONF_IDENTITY,
    CONF_ROLE,
    CONF_SECRET,
    DOMAIN,
)
import wellfed_homeassistant as bridge

from . import fake

WAIT = 10


@pytest.fixture
def feeder(monkeypatch: pytest.MonkeyPatch) -> Iterator[fake.FakeFeeder]:
    """The scripted bridge every session in the test gets."""
    scripted = fake.FakeFeeder()

    def make(secret: bytes, identity: bytes, address: str | None) -> fake.FakeFeeder:
        return scripted

    monkeypatch.setattr(bridge, "Feeder", make)
    yield scripted


async def _until(hass: HomeAssistant, condition: Callable[[], bool]) -> None:
    async with asyncio.timeout(WAIT):
        while not condition():
            await asyncio.sleep(0.05)
            await hass.async_block_till_done()


def _entry(hass: HomeAssistant, role: str = "owner", **options: Any) -> MockConfigEntry:
    entry = MockConfigEntry(
        domain=DOMAIN,
        unique_id=fake.IDENTITY.hex(),
        data={
            CONF_SECRET: fake.SECRET.hex(),
            CONF_IDENTITY: fake.IDENTITY.hex(),
            CONF_ADDRESS: "192.0.2.1:4747",
            CONF_ROLE: role,
        },
        options=options,
    )
    entry.add_to_hass(hass)
    return entry


async def _ready(
    hass: HomeAssistant, feeder: fake.FakeFeeder, role: str = "owner", **options: Any
) -> MockConfigEntry:
    bridge_role = bridge.Role.Owner if role == "owner" else bridge.Role.Member
    feeder.push(
        fake.connected(bridge_role), fake.update(bridge.UpdateKind.State, state=fake.state())
    )
    entry = _entry(hass, role, **options)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    return entry


def _issue(hass: HomeAssistant, issue: str) -> ir.IssueEntry | None:
    return ir.async_get(hass).async_get_issue(DOMAIN, f"{issue}_{fake.IDENTITY.hex()}")


async def test_a_lost_feed_opens_a_repair_its_settlement_clears(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data

    feeder.feed_result = fake.written(bridge.WriteResult.Uncertain)
    with pytest.raises(HomeAssistantError) as raised:
        await session.feed(1)
    assert raised.value.translation_key == "write_uncertain"
    assert _issue(hass, "uncertain_feed") is not None

    feeder.push(fake.update(bridge.UpdateKind.Settled, feed=fake.feed(4)))
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is None)
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_with_no_record_is_reported_unknown(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    feeder.feed_result = fake.written(bridge.WriteResult.Uncertain)
    with pytest.raises(HomeAssistantError):
        await entry.runtime_data.feed(2)

    feeder.push(fake.update(bridge.UpdateKind.StillUnknown, portions=2))
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)
    unknown = _issue(hass, "unknown_feed")
    assert unknown is not None and unknown.translation_placeholders is not None
    assert unknown.translation_placeholders["portions"] == "2"
    assert _issue(hass, "uncertain_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


@pytest.mark.parametrize(
    ("written", "key"),
    [
        (fake.written(bridge.WriteResult.NotSent, not_sent=bridge.NotSent.Busy), "not_sent_busy"),
        (
            fake.written(bridge.WriteResult.Refused, refusal=bridge.RefusalKind.Portions),
            "refused_portions",
        ),
    ],
)
async def test_a_write_that_did_not_happen_says_why(
    hass: HomeAssistant, feeder: fake.FakeFeeder, written: Any, key: str
) -> None:
    entry = await _ready(hass, feeder)
    feeder.feed_result = written
    with pytest.raises(HomeAssistantError) as raised:
        await entry.runtime_data.feed(1)
    assert raised.value.translation_key == key
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_key_revoked_before_setup_asks_to_pair_again(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    feeder.push(fake.update(bridge.UpdateKind.Revoked))
    entry = _entry(hass)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert entry.state is ConfigEntryState.SETUP_ERROR
    flows = hass.config_entries.flow.async_progress_by_handler(DOMAIN)
    assert [flow["context"]["source"] for flow in flows] == ["reauth"]


async def test_a_feeder_that_never_answers_is_tried_again_later(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("custom_components.wellfed.READY_WAIT", 0.2)
    entry = _entry(hass)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    assert entry.state is ConfigEntryState.SETUP_RETRY


async def test_the_firmware_entity_offers_and_installs_a_release(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    feeder.offer = fake.OFFER
    entry = await _ready(hass, feeder, project_keys=["aa" * 32])
    firmware = f"update.feeder_{fake.IDENTITY.hex()[:8]}_firmware"
    state = hass.states.get(firmware)
    assert state is not None and state.state == STATE_ON
    assert state.attributes["latest_version"] == fake.OFFER.build[:12]
    assert state.attributes["release_summary"] == "Release 9 (v9)"
    assert (
        "newest_release",
        (
            str(Path(hass.config.path(DOMAIN, fake.IDENTITY.hex()))),
            [bytes.fromhex("aa" * 32)],
            True,
        ),
    ) in feeder.calls

    await hass.services.async_call("update", "install", {ATTR_ENTITY_ID: firmware}, blocking=True)
    assert any(call[0] == "install" and call[1][2] == fake.OFFER.id for call in feeder.calls)
    await _until(hass, lambda: entry.runtime_data.install_progress is not None)

    feeder.install_result = SimpleNamespace(
        result=bridge.Installed.Failed, offered=True, not_sent=None
    )
    with pytest.raises(HomeAssistantError) as raised:
        await hass.services.async_call(
            "update", "install", {ATTR_ENTITY_ID: firmware}, blocking=True
        )
    assert raised.value.translation_key == "install_failed"
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_with_no_release_held_the_firmware_is_current(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    firmware = f"update.feeder_{fake.IDENTITY.hex()[:8]}_firmware"
    state = hass.states.get(firmware)
    assert state is not None and state.state == STATE_OFF
    assert (
        "newest_release",
        (str(Path(hass.config.path(DOMAIN, fake.IDENTITY.hex()))), [], False),
    ) in feeder.calls
    with pytest.raises(HomeAssistantError):
        await hass.services.async_call(
            "update", "install", {ATTR_ENTITY_ID: firmware}, blocking=True
        )
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_actions_cancel_a_named_feed_and_add_an_allowed_release(
    hass: HomeAssistant, feeder: fake.FakeFeeder, tmp_path: Path
) -> None:
    entry = await _ready(hass, feeder)
    device = dr.async_get(hass).async_get_device_by_identifier(
        (DOMAIN, fake.IDENTITY.hex()), entry.entry_id
    )
    assert device is not None

    await hass.services.async_call(
        DOMAIN, "cancel_feed", {"device_id": device.id, "feed": 7}, blocking=True
    )
    assert ("cancel_feed", (7,)) in feeder.calls

    release = tmp_path / "feeder.plota"
    release.write_bytes(b"release")
    hass.config.allowlist_external_dirs = {str(tmp_path)}
    added = await hass.services.async_call(
        DOMAIN,
        "add_release",
        {"device_id": device.id, "path": str(release)},
        blocking=True,
        return_response=True,
    )
    assert added == {"version": 9, "build": fake.OFFER.build}
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_member_may_not_add_a_release(hass: HomeAssistant, feeder: fake.FakeFeeder) -> None:
    entry = await _ready(hass, feeder, role="member")
    device = dr.async_get(hass).async_get_device_by_identifier(
        (DOMAIN, fake.IDENTITY.hex()), entry.entry_id
    )
    assert device is not None
    with pytest.raises(HomeAssistantError) as raised:
        await hass.services.async_call(
            DOMAIN, "add_release", {"device_id": device.id, "path": "/x.plota"}, blocking=True
        )
    assert raised.value.translation_key == "owner_only"
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_diagnostics_redact_the_key(
    hass: HomeAssistant, feeder: fake.FakeFeeder, hass_client: ClientSessionGenerator
) -> None:
    entry = await _ready(hass, feeder, project_keys=["aa" * 32])
    diagnostics = await get_diagnostics_for_config_entry(hass, hass_client, entry)
    assert diagnostics["entry"][CONF_SECRET] == "**REDACTED**"
    assert fake.SECRET.hex() not in str(diagnostics)
    assert diagnostics["role"] == "owner"
    assert diagnostics["status"]["build"] == "ab" * 32
    assert diagnostics["capabilities"]["update"] is True
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_only_a_device_that_is_not_the_feeder_may_be_removed(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    registry = dr.async_get(hass)
    current = registry.async_get_device_by_identifier((DOMAIN, fake.IDENTITY.hex()), entry.entry_id)
    assert current is not None
    stale = registry.async_get_or_create(
        config_entry_id=entry.entry_id, identifiers={(DOMAIN, "00" * 32)}
    )
    assert not await async_remove_config_entry_device(hass, entry, current)
    assert await async_remove_config_entry_device(hass, entry, stale)
    assert await hass.config_entries.async_unload(entry.entry_id)
