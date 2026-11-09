"""Paths the simulator cannot reach from Python, on a scripted bridge: a
feed whose answer was lost opens a repair that its settlement clears, a
revoked key, a feeder of another build or a silent feeder at setup,
diagnostics and stale devices."""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Iterator
from typing import Any

from homeassistant.config_entries import ConfigEntry, ConfigEntryState
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


def _entry(hass: HomeAssistant) -> MockConfigEntry:
    entry = MockConfigEntry(
        domain=DOMAIN,
        unique_id=fake.IDENTITY.hex(),
        data={
            CONF_SECRET: fake.SECRET.hex(),
            CONF_IDENTITY: fake.IDENTITY.hex(),
            CONF_ADDRESS: "192.0.2.1:4747",
        },
    )
    entry.add_to_hass(hass)
    return entry


async def _ready(hass: HomeAssistant, feeder: fake.FakeFeeder) -> MockConfigEntry:
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    entry = _entry(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    return entry


def _entry_state(entry: ConfigEntry) -> ConfigEntryState:
    """The entry's state now: read through a call, since a reload changes
    what an earlier assertion narrowed it to."""
    return entry.state


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


async def test_a_feeder_of_another_revision_is_served_and_names_the_side_to_update(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    feeder.push(
        fake.connected(skew=bridge.Skew.FeederBehind),
        fake.update(bridge.UpdateKind.State, state=fake.state()),
    )
    entry = _entry(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert _entry_state(entry) is ConfigEntryState.LOADED
    issue = _issue(hass, "update_feeder")
    assert issue is not None and issue.translation_placeholders is not None
    assert issue.translation_placeholders["build"] == "ab" * 32
    assert _issue(hass, "update_integration") is None

    # A newer feeder build names this integration instead.
    feeder.push(fake.connected(skew=bridge.Skew.FeederAhead))
    await _until(hass, lambda: _issue(hass, "update_integration") is not None)
    assert _issue(hass, "update_feeder") is None

    # Once both match, neither stands.
    feeder.push(fake.connected())
    await _until(hass, lambda: _issue(hass, "update_integration") is None)
    assert _issue(hass, "update_feeder") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_pre_revision_feeder_asks_for_a_feeder_update_and_is_retried(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("custom_components.wellfed.READY_WAIT", 0.2)
    # The bridge says so on connect, then reads nothing: no state comes.
    feeder.push(fake.connected(skew=bridge.Skew.Legacy))
    entry = _entry(hass)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert entry.state is ConfigEntryState.SETUP_RETRY
    assert _issue(hass, "legacy_feeder") is not None
    assert _issue(hass, "update_feeder") is None

    # Updated from the app, it is set up as any other and the repair goes.
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    assert await hass.config_entries.async_reload(entry.entry_id)
    await hass.async_block_till_done()
    assert _entry_state(entry) is ConfigEntryState.LOADED
    assert _issue(hass, "legacy_feeder") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_feeder_that_never_answers_is_tried_again_later(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("custom_components.wellfed.READY_WAIT", 0.2)
    entry = _entry(hass)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    assert entry.state is ConfigEntryState.SETUP_RETRY


async def test_diagnostics_redact_the_key(
    hass: HomeAssistant, feeder: fake.FakeFeeder, hass_client: ClientSessionGenerator
) -> None:
    entry = await _ready(hass, feeder)
    diagnostics = await get_diagnostics_for_config_entry(hass, hass_client, entry)
    assert diagnostics["entry"][CONF_SECRET] == "**REDACTED**"
    assert fake.SECRET.hex() not in str(diagnostics)
    assert diagnostics["role"] == "member"
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


async def test_the_cancel_action_stops_a_named_feed(
    hass: HomeAssistant, feeder: fake.FakeFeeder
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
    assert await hass.config_entries.async_unload(entry.entry_id)
