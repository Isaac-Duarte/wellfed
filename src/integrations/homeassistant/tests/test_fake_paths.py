"""Paths the simulator and the server cannot reach from Python, on a
scripted bridge: a feed whose answer was lost opens a repair that its
settlement clears, and is kept across a reload; a revoked key or token, a
feeder of another build, a server that lost its feeder or a silent feeder
at setup, an entry of a newer integration or from before the routes,
diagnostics, stale devices, write entities hidden from a role that writes
nothing, the repairs a token or key left behind, and the frozen unique ids
on both routes."""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Iterator
import json
import os
from pathlib import Path
import threading
from typing import Any, cast

from homeassistant.config_entries import (
    SOURCE_REAUTH,
    SOURCE_RECONFIGURE,
    ConfigEntry,
    ConfigEntryState,
)
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import (
    device_registry as dr,
    entity_registry as er,
    issue_registry as ir,
)
from homeassistant.setup import async_setup_component
import pytest
from pytest_homeassistant_custom_component.common import MockConfigEntry
from pytest_homeassistant_custom_component.components.diagnostics import (
    get_diagnostics_for_config_entry,
)
from pytest_homeassistant_custom_component.typing import ClientSessionGenerator

from custom_components.wellfed import async_remove_config_entry_device
from custom_components.wellfed.const import (
    CONF_ADDRESS,
    CONF_FEEDER_ID,
    CONF_IDENTITY,
    CONF_ROUTE,
    CONF_SECRET,
    CONF_TOKEN,
    CONF_TOKEN_ID,
    CONF_URL,
    DOMAIN,
    ROUTE_DIRECT,
    ROUTE_SERVER,
)
from custom_components.wellfed.kept import kept
from custom_components.wellfed.session import key_left
import wellfed_homeassistant as bridge

from . import fake

WAIT = 10


class _Make:
    """Stands in for the bridge's `Feeder` class: either route's session is the scripted one."""

    def __init__(self, scripted: fake.FakeFeeder) -> None:
        self.scripted = scripted
        self.routes: list[str] = []
        # What each session was handed of the last run's unsettled feeds.
        self.handed: list[tuple[list[str], list[int]]] = []

    def __call__(
        self,
        secret: bytes,
        identity: bytes,
        address: str | None,
        pending: list[bridge.Pending],
        answered: list[int],
    ) -> fake.FakeFeeder:
        self.routes.append(ROUTE_DIRECT)
        self.handed.append(([p.id for p in pending], answered))
        return self.scripted

    def server(
        self,
        url: str,
        token: str,
        feeder_id: str,
        identity: bytes,
        pending: list[bridge.Pending],
    ) -> fake.FakeFeeder:
        self.routes.append(ROUTE_SERVER)
        self.handed.append(([p.id for p in pending], []))
        return self.scripted


@pytest.fixture
def feeder(monkeypatch: pytest.MonkeyPatch) -> Iterator[fake.FakeFeeder]:
    """The scripted bridge every session in the test gets."""
    scripted = fake.FakeFeeder()
    monkeypatch.setattr(bridge, "Feeder", _Make(scripted))
    yield scripted


async def _until(hass: HomeAssistant, condition: Callable[[], bool]) -> None:
    async with asyncio.timeout(WAIT):
        while not condition():
            await asyncio.sleep(0.05)
            await hass.async_block_till_done()


# Version 2 of an entry's data, on either route.
DIRECT_DATA = {
    CONF_ROUTE: ROUTE_DIRECT,
    CONF_SECRET: fake.SECRET.hex(),
    CONF_IDENTITY: fake.IDENTITY.hex(),
    CONF_ADDRESS: "192.0.2.1:4747",
}
SERVER_DATA = {
    CONF_ROUTE: ROUTE_SERVER,
    CONF_IDENTITY: fake.IDENTITY.hex(),
    CONF_URL: "http://192.0.2.2:8080",
    CONF_TOKEN: "a-token-secret",
    CONF_TOKEN_ID: "01a0c450-6c00-7db9-b536-8a8beb004413",
    CONF_FEEDER_ID: "01a0c450-6c00-7b52-8b49-e2dbe147dc4a",
}


def _entry(
    hass: HomeAssistant, data: dict[str, Any] = DIRECT_DATA, version: int = 2
) -> MockConfigEntry:
    entry = MockConfigEntry(
        domain=DOMAIN, unique_id=fake.IDENTITY.hex(), data=data, version=version
    )
    entry.add_to_hass(hass)
    return entry


async def _ready(
    hass: HomeAssistant,
    feeder: fake.FakeFeeder,
    data: dict[str, Any] = DIRECT_DATA,
    state: Any = None,
) -> MockConfigEntry:
    feeder.push(
        fake.connected(),
        fake.update(bridge.UpdateKind.State, state=state or fake.state()),
    )
    entry = _entry(hass, data)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    return entry


def _entry_state(entry: ConfigEntry) -> ConfigEntryState:
    """The entry's state now: read through a call, since a reload changes
    what an earlier assertion narrowed it to."""
    return entry.state


def _issue(hass: HomeAssistant, issue: str) -> ir.IssueEntry | None:
    return ir.async_get(hass).async_get_issue(DOMAIN, f"{issue}_{fake.IDENTITY.hex()}")


async def test_a_plan_table_not_read_removes_no_plan_switch(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder, state=fake.state(plans=[fake.plan(0)]))
    registry = er.async_get(hass)
    shown = registry.async_get_entity_id("switch", DOMAIN, f"{fake.IDENTITY.hex()}-plan_0")
    assert shown is not None
    registry.async_update_entity(shown, new_entity_id="switch.breakfast")

    # A state with the table unread, as a session shows before it reads it
    # again: the owner's switch stays, unavailable, under its new id.
    feeder.push(fake.update(bridge.UpdateKind.State, state=fake.state(plans=None)))
    session = entry.runtime_data
    await _until(hass, lambda: session.state is not None and session.state.plans is None)
    assert registry.async_get("switch.breakfast") is not None
    breakfast = hass.states.get("switch.breakfast")
    assert breakfast is not None and breakfast.state == "unavailable"

    feeder.push(fake.update(bridge.UpdateKind.State, state=fake.state(plans=[fake.plan(0)])))
    await _until(hass, lambda: getattr(hass.states.get("switch.breakfast"), "state", None) == "on")
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_opens_a_repair_its_settlement_clears(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data

    lost = fake.pending()
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError) as raised:
        await session.feed(1)
    assert raised.value.translation_key == "write_uncertain"
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is not None)

    feeder.push(fake.update(bridge.UpdateKind.Settled, feed=fake.feed(4), pending=lost))
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is None)
    assert session.kept.pending == []
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_is_kept_across_a_reload_and_settled_by_the_next_session(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    # Answered with nothing pending: kept all the same, since its `Fed` may
    # come after a feed lost next.
    feeder.feed_result = fake.written(bridge.WriteResult.Feeding, feed=41)
    assert await entry.runtime_data.feed(1) == 41
    lost = fake.pending()
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError):
        await entry.runtime_data.feed(1)
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is not None)
    # Answered after it went out: the next session must know.
    feeder.feed_result = fake.written(bridge.WriteResult.Feeding, feed=9)
    assert await entry.runtime_data.feed(1) == 9

    assert await hass.config_entries.async_unload(entry.entry_id)
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()

    # The fixture put its maker in the bridge's place.
    made = cast(_Make, bridge.Feeder)
    assert made.handed[-1] == ([lost.id], [41, 9])
    # The bridge announces again a lost feed that still holds feeds back.
    feeder.push(fake.update(bridge.UpdateKind.Pending, pending=lost))
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is not None)
    feeder.push(fake.update(bridge.UpdateKind.SettledUnfed, pending=lost))
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is None)
    assert entry.runtime_data.kept.pending == []
    assert entry.runtime_data.kept.answered == [41, 9]
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_settled_before_its_write_returns_is_not_kept(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending()
    # The bridge announces it and the journal settles it before the write's
    # own answer reaches the event loop.
    feeder.push(
        fake.update(bridge.UpdateKind.Pending, pending=lost),
        fake.update(bridge.UpdateKind.SettledUnfed, pending=lost),
    )
    await _until(hass, lambda: feeder.updates.empty())
    await hass.async_block_till_done()
    # Its pending update came already; the write's answer carries no other.
    feeder.feed_result = fake.written(bridge.WriteResult.Uncertain)
    with pytest.raises(HomeAssistantError):
        await session.feed(1)
    await hass.async_block_till_done()

    assert session.kept.pending == []
    assert _issue(hass, "uncertain_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_settled_by_its_fed_keeps_its_number(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending()
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError):
        await session.feed(1)

    # The bridge announces the number before the settlement: after a
    # restart, the replay of this `Fed` must settle no other lost feed.
    feeder.push(
        fake.update(bridge.UpdateKind.Answered, answered=[4]),
        fake.update(bridge.UpdateKind.Settled, feed=fake.feed(4), pending=lost),
    )
    # In order: the lost feed is kept, its number comes, then it settles.
    await _until(hass, lambda: session.kept.answered == [4])
    await _until(hass, lambda: session.kept.pending == [])
    assert _issue(hass, "uncertain_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_reload_waits_for_a_feed_still_out_and_hands_its_loss_on(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending()
    feeder.gate = threading.Event()
    feeder.feed_result = fake.uncertain(lost)
    pressed = hass.async_create_task(session.feed(1))
    try:
        # Not `_until`: waiting for every task would wait for this feed.
        async with asyncio.timeout(WAIT):
            while ("feed", (1,)) not in feeder.calls:
                await asyncio.sleep(0.05)

        # The reload starts while the feed is out, and waits for it: its
        # answer is lost only after.
        made = cast(_Make, bridge.Feeder)
        sessions = len(made.handed)
        reload = hass.async_create_task(hass.config_entries.async_reload(entry.entry_id))
        await asyncio.sleep(0.2)
        assert not reload.done(), "the reload waits for the write"
    finally:
        feeder.gate.set()
    with pytest.raises(HomeAssistantError):
        await pressed

    # The next session is handed the lost feed, which holds its new feeds.
    async with asyncio.timeout(WAIT):
        while len(made.handed) == sessions:
            await asyncio.sleep(0.05)
    assert made.handed[-1] == ([lost.id], [])
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    assert await reload
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_stop_keeps_a_lost_feed_s_updates_in_their_order(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending()
    # The session learns the whole story just as the entry unloads: the
    # pump hands the event loop its start once the stop has begun, and the
    # rest still waits for the drain. Kept the other way round, the
    # settlement would come before the feed it settles, and the feed would
    # stay lost.
    feeder.at_stop(
        session._stop,
        fake.update(bridge.UpdateKind.Pending, pending=lost),
        fake.update(bridge.UpdateKind.Answered, answered=[4]),
        fake.update(bridge.UpdateKind.Settled, feed=fake.feed(4), pending=lost),
    )
    assert await hass.config_entries.async_unload(entry.entry_id)
    assert not feeder.held, "the pump handed one, and the drain the rest"
    await hass.async_block_till_done()

    assert session.kept.pending == [], "settled after it was kept, never before"
    assert session.kept.answered == [4]


async def test_a_store_in_another_shape_is_set_aside_and_setup_goes_on(
    hass: HomeAssistant,
    feeder: fake.FakeFeeder,
    hass_storage: dict[str, Any],
    caplog: pytest.LogCaptureFixture,
) -> None:
    entry = _entry(hass)
    hass_storage[f"{DOMAIN}.{entry.entry_id}"] = {
        "version": 1,
        "minor_version": 1,
        "key": f"{DOMAIN}.{entry.entry_id}",
        "data": {"pending": [{"id": "x"}]},
    }
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert entry.runtime_data.kept.pending == []
    assert "not in the shape it writes" in caplog.text
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_with_no_record_is_reported_unknown(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    lost = fake.pending(portions=2)
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError):
        await entry.runtime_data.feed(2)

    feeder.push(fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost))
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)
    unknown = _issue(hass, "unknown_feed")
    assert unknown is not None and unknown.translation_placeholders is not None
    assert unknown.translation_placeholders["portions"] == "2"
    # It still holds feeds back, and its repair is the one that releases it.
    assert unknown.is_fixable
    assert _issue(hass, "uncertain_feed") is None
    assert [p.id for p in entry.runtime_data.kept.pending] == [lost.id]
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_repair_releases_a_lost_feed_and_says_it_may_have_run(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending(portions=2)
    feeder.push(
        fake.update(bridge.UpdateKind.Pending, pending=lost),
        fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost),
    )
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)
    unknown = _issue(hass, "unknown_feed")
    assert unknown is not None

    assert await async_setup_component(hass, "repairs", {})
    flows = hass.data["repairs"]["flow_manager"]
    shown = await flows.async_init(DOMAIN, context={"issue_id": unknown.issue_id})
    assert shown["type"] == FlowResultType.FORM
    assert shown["step_id"] == "confirm"
    assert shown["description_placeholders"]["portions"] == "2"
    assert ("release", (lost.id,)) not in feeder.calls, "only once submitted"

    done = await flows.async_configure(shown["flow_id"], {})
    assert done["type"] == FlowResultType.CREATE_ENTRY
    assert ("release", (lost.id,)) in feeder.calls
    assert _issue(hass, "unknown_feed") is None

    # The bridge confirms it: kept released, so the next session does not
    # hold on it, and still waited on for its record.
    released = bridge.Pending(lost.id, 2, lost.sent_ms, None, True, 1_793_794_000_000)
    feeder.push(fake.update(bridge.UpdateKind.Released, portions=2, pending=released))
    await _until(hass, lambda: [p.released_ms for p in session.kept.pending] == [1_793_794_000_000])
    assert _issue(hass, "unknown_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_repair_on_an_unloaded_entry_releases_nothing(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    lost = fake.pending(portions=2)
    feeder.push(
        fake.update(bridge.UpdateKind.Pending, pending=lost),
        fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost),
    )
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)
    unknown = _issue(hass, "unknown_feed")
    assert unknown is not None

    assert await async_setup_component(hass, "repairs", {})
    flows = hass.data["repairs"]["flow_manager"]
    shown = await flows.async_init(DOMAIN, context={"issue_id": unknown.issue_id})
    assert await hass.config_entries.async_unload(entry.entry_id)
    done = await flows.async_configure(shown["flow_id"], {})
    assert done["type"] == FlowResultType.ABORT
    assert done["reason"] == "not_loaded"
    assert ("release", (lost.id,)) not in feeder.calls


async def test_through_the_server_a_lost_feed_holds_nothing_and_has_no_release(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    feeder.holds_lost_feeds = False
    entry = await _ready(hass, feeder, SERVER_DATA)
    session = entry.runtime_data
    lost = fake.pending(portions=2)
    feeder.push(
        fake.update(bridge.UpdateKind.Pending, pending=lost),
        fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost),
    )
    await _until(hass, lambda: _issue(hass, "unknown_feed_server") is not None)
    unknown = _issue(hass, "unknown_feed_server")
    assert unknown is not None and not unknown.is_fixable
    assert _issue(hass, "unknown_feed") is None

    feeder.push(fake.update(bridge.UpdateKind.Forgotten, portions=2, pending=lost))
    await _until(hass, lambda: session.kept.pending == [])
    # Still unknown: the owner's repair stays until they ignore it.
    assert _issue(hass, "unknown_feed_server") is not None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_reported_unknown_is_settled_by_a_late_record(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending(portions=2)
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError):
        await session.feed(2)
    feeder.push(fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost))
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)

    # Its `Fed` turns up after all: the feed is known, and both repairs go.
    feeder.push(fake.update(bridge.UpdateKind.Settled, feed=fake.feed(4), pending=lost))
    await _until(hass, lambda: _issue(hass, "unknown_feed") is None)
    assert _issue(hass, "uncertain_feed") is None
    assert session.kept.pending == []
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_no_longer_waited_on_leaves_the_store(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending(portions=2)
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError):
        await session.feed(2)
    feeder.push(fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost))
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)
    assert [p.id for p in session.kept.pending] == [lost.id]

    # The bridge forgets only a feed released a week before.
    feeder.push(fake.update(bridge.UpdateKind.Forgotten, portions=2, pending=lost))
    await _until(hass, lambda: session.kept.pending == [])
    assert _issue(hass, "unknown_feed") is None
    assert _issue(hass, "uncertain_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_feed_going_out_is_kept_with_no_repair_until_its_answer(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    out = fake.pending("out", portions=2)
    feeder.push(fake.update(bridge.UpdateKind.Sending, pending=out))
    await _until(hass, lambda: [p.id for p in session.kept.pending] == ["out"])
    # Only a lost answer says it may have run.
    assert _issue(hass, "uncertain_feed") is None

    feeder.push(fake.update(bridge.UpdateKind.Cleared, pending=out))
    await _until(hass, lambda: session.kept.pending == [])
    assert _issue(hass, "uncertain_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_feed_reported_unknown_is_kept_as_reported(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    session = entry.runtime_data
    lost = fake.pending(portions=2)
    feeder.push(
        fake.update(bridge.UpdateKind.Pending, pending=lost),
        fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=lost),
    )
    await _until(hass, lambda: _issue(hass, "unknown_feed") is not None)
    # The next session is handed it as reported, and reports it no more.
    assert [(p.id, p.reported) for p in session.kept.pending] == [(lost.id, True)]
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_reported_lost_feed_keeps_no_repair_for_a_newer_one(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    older, newer = fake.pending("older", portions=2), fake.pending("newer", portions=1)
    # The older one was reported still unknown; the newer one still holds
    # feeds back until it is settled.
    feeder.push(
        fake.update(bridge.UpdateKind.Pending, pending=older),
        fake.update(bridge.UpdateKind.StillUnknown, portions=2, pending=older),
        fake.update(bridge.UpdateKind.Pending, pending=newer),
    )
    await _until(hass, lambda: len(entry.runtime_data.kept.pending) == 2)
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is not None)
    feeder.push(fake.update(bridge.UpdateKind.Settled, feed=fake.feed(4), pending=newer))
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is None)
    # The older one is still waited on, and still unknown.
    assert [p.id for p in entry.runtime_data.kept.pending] == [older.id]
    assert _issue(hass, "unknown_feed") is not None
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


async def test_a_token_refused_before_setup_asks_to_sign_in_again(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    feeder.push(fake.update(bridge.UpdateKind.TokenRefused))
    entry = _entry(hass, SERVER_DATA)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    assert entry.state is ConfigEntryState.SETUP_ERROR
    flows = hass.config_entries.flow.async_progress_by_handler(DOMAIN)
    assert [flow["context"]["source"] for flow in flows] == ["reauth"]
    assert flows[0]["step_id"] == "reauth_server"


async def test_a_server_that_lost_its_feeder_opens_a_repair_until_it_reaches_it(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder, SERVER_DATA)
    feeder.push(fake.update(bridge.UpdateKind.ServerLostFeeder, reason="removed"))
    await _until(hass, lambda: _issue(hass, "server_lost_feeder") is not None)
    assert not entry.runtime_data.connected

    feeder.push(fake.connected())
    await _until(hass, lambda: _issue(hass, "server_lost_feeder") is None)
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_an_older_server_names_the_server_and_a_newer_one_this_integration(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder, SERVER_DATA)
    feeder.push(fake.connected(skew=bridge.Skew.ServerBehind))
    await _until(hass, lambda: _issue(hass, "update_server") is not None)

    feeder.push(fake.connected(skew=bridge.Skew.ServerAhead))
    await _until(hass, lambda: _issue(hass, "update_integration") is not None)
    assert _issue(hass, "update_server") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_lost_feed_refused_after_all_clears_its_repair(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder, SERVER_DATA)
    lost = fake.pending()
    feeder.feed_result = fake.uncertain(lost)
    with pytest.raises(HomeAssistantError):
        await entry.runtime_data.feed(1)
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is not None)

    # The server's journal found the feeder refused it: nothing ran.
    feeder.push(fake.update(bridge.UpdateKind.SettledUnfed, pending=lost))
    await _until(hass, lambda: _issue(hass, "uncertain_feed") is None)
    assert _issue(hass, "unknown_feed") is None
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_write_to_a_pre_revision_feeder_says_to_update_it(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder, SERVER_DATA)
    feeder.feed_result = fake.written(bridge.WriteResult.NotSent, not_sent=bridge.NotSent.Legacy)
    with pytest.raises(HomeAssistantError) as raised:
        await entry.runtime_data.feed(1)
    assert raised.value.translation_key == "not_sent_legacy"
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_an_entry_of_a_newer_integration_is_refused(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = _entry(hass, version=3)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    assert entry.state is ConfigEntryState.MIGRATION_ERROR


async def test_an_entry_of_a_newer_minor_is_read_as_it_is(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = MockConfigEntry(
        domain=DOMAIN,
        unique_id=fake.IDENTITY.hex(),
        data={**DIRECT_DATA, "added_by_a_newer_minor": True},
        version=2,
        minor_version=2,
    )
    entry.add_to_hass(hass)
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))

    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()

    assert entry.state is ConfigEntryState.LOADED
    assert (entry.version, entry.minor_version) == (2, 2), "left at its minor"
    assert entry.data["added_by_a_newer_minor"] is True
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_an_entry_from_before_the_routes_fails_to_migrate_and_deletes_cleanly(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    # Version 1: a direct entry with no route.
    old = {key: value for key, value in DIRECT_DATA.items() if key != CONF_ROUTE}
    entry = _entry(hass, old, version=1)
    assert not await hass.config_entries.async_setup(entry.entry_id)
    assert entry.state is ConfigEntryState.MIGRATION_ERROR

    for source in (SOURCE_RECONFIGURE, SOURCE_REAUTH):
        result = await hass.config_entries.flow.async_init(
            DOMAIN, context={"source": source, "entry_id": entry.entry_id}, data=entry.data
        )
        assert result["type"] is FlowResultType.ABORT
        assert result["reason"] == "pair_again"
    assert await hass.config_entries.async_remove(entry.entry_id)


async def test_diagnostics_redact_the_token(
    hass: HomeAssistant, feeder: fake.FakeFeeder, hass_client: ClientSessionGenerator
) -> None:
    entry = await _ready(hass, feeder, SERVER_DATA)
    diagnostics = await get_diagnostics_for_config_entry(hass, hass_client, entry)
    assert diagnostics["entry"][CONF_TOKEN] == "**REDACTED**"
    assert "a-token-secret" not in str(diagnostics)
    assert await hass.config_entries.async_unload(entry.entry_id)


# Every entity and device id a feeder has ever had, with `{identity}` for
# its identity key. Ids are frozen: an entity's history and its automations
# hang on them, on either route. A new entity appends its id here; none is
# ever renamed or removed.
UNIQUE_IDS = Path(__file__).with_name("fixtures") / "unique_ids.json"


def _ids(hass: HomeAssistant, entry: ConfigEntry) -> dict[str, list[str]]:
    identity = fake.IDENTITY.hex()
    entities = er.async_entries_for_config_entry(er.async_get(hass), entry.entry_id)
    devices = dr.async_entries_for_config_entry(dr.async_get(hass), entry.entry_id)
    return {
        "entities": sorted(e.unique_id.replace(identity, "{identity}") for e in entities),
        "devices": sorted(
            f"{domain}:{value.replace(identity, '{identity}')}"
            for device in devices
            for domain, value in device.identifiers
        ),
    }


@pytest.mark.parametrize("data", [DIRECT_DATA, SERVER_DATA], ids=["direct", "server"])
async def test_the_unique_ids_are_frozen_and_the_same_on_both_routes(
    hass: HomeAssistant, feeder: fake.FakeFeeder, data: dict[str, Any]
) -> None:
    plans = [fake.plan(slot) for slot in (0, 9)]
    entry = await _ready(hass, feeder, data, fake.state(plans=plans))
    current = _ids(hass, entry)
    frozen: dict[str, list[str]] = json.loads(UNIQUE_IDS.read_text())

    # Append-only: an id renamed or removed loses its entity's history, and
    # no blessing stores that. A new one is blessed (`WELLFED_BLESS=1`) by
    # appending it after the ids already frozen, which keep their order.
    blessing = os.environ.get("WELLFED_BLESS") == "1"
    for kind in ("entities", "devices"):
        gone = [unique_id for unique_id in frozen[kind] if unique_id not in current[kind]]
        assert not gone, f"{kind} renamed or removed: their history is lost: {gone}"
        added = [unique_id for unique_id in current[kind] if unique_id not in frozen[kind]]
        if blessing:
            frozen[kind] = [*frozen[kind], *added]
        else:
            assert not added, f"new {kind}: bless them with WELLFED_BLESS=1: {added}"
    if blessing:
        UNIQUE_IDS.write_text(json.dumps(frozen, indent=2) + "\n")
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_deleting_a_server_entry_revokes_its_token(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    revoked: list[tuple[str, str, str]] = []
    monkeypatch.setattr(
        bridge,
        "server_revoke_token",
        lambda url, token, token_id: revoked.append((url, token, token_id)),
    )
    entry = await _ready(hass, feeder, SERVER_DATA)
    assert await hass.config_entries.async_remove(entry.entry_id)
    assert revoked == [(SERVER_DATA[CONF_URL], SERVER_DATA[CONF_TOKEN], SERVER_DATA[CONF_TOKEN_ID])]


async def test_a_token_the_server_could_not_revoke_is_named_in_a_repair(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    def unreachable(url: str, token: str, token_id: str) -> None:
        raise bridge.WellfedError("the server cannot be reached")

    monkeypatch.setattr(bridge, "server_revoke_token", unreachable)
    entry = await _ready(hass, feeder, SERVER_DATA)
    assert await hass.config_entries.async_remove(entry.entry_id)

    token_id = SERVER_DATA[CONF_TOKEN_ID]
    issue = ir.async_get(hass).async_get_issue(DOMAIN, f"token_left_{token_id}")
    assert issue is not None and issue.is_persistent
    # Named by its id, never by its secret.
    assert issue.translation_placeholders == {"url": SERVER_DATA[CONF_URL], "token_id": token_id}


async def test_a_leftover_key_clears_once_gone_and_with_its_entry(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    listed = [True]
    monkeypatch.setattr(
        bridge, "server_client_listed", lambda url, token, feeder_id, key: listed[0]
    )
    monkeypatch.setattr(bridge, "server_revoke_token", lambda url, token, token_id: None)

    def leftover() -> None:
        ir.async_create_issue(
            hass,
            DOMAIN,
            f"leftover_key_{fake.IDENTITY.hex()}",
            is_fixable=False,
            is_persistent=True,
            severity=ir.IssueSeverity.WARNING,
            translation_key="leftover_key",
            translation_placeholders={"feeder": "f", "key": "k"},
        )

    entry = _entry(hass, SERVER_DATA)
    store = kept(hass, entry.entry_id)
    await store.load()
    store.set_leftover_key(bytes(32))
    leftover()
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    # The feeder still lists it.
    assert _issue(hass, "leftover_key") is not None

    listed[0] = False
    feeder.push(fake.connected())
    await _until(hass, lambda: _issue(hass, "leftover_key") is None)
    assert store.leftover_key is None

    leftover()
    assert await hass.config_entries.async_remove(entry.entry_id)
    assert _issue(hass, "leftover_key") is None


async def test_a_leftover_key_is_checked_directly_and_outlives_its_entry(
    hass: HomeAssistant, feeder: fake.FakeFeeder, monkeypatch: pytest.MonkeyPatch
) -> None:
    listed = [True]
    asked: list[tuple[bytes, bytes, bytes, str | None]] = []

    def client_listed(
        secret: bytes, identity: bytes, key: bytes, address: str | None = None
    ) -> bool:
        asked.append((secret, identity, key, address))
        return listed[0]

    monkeypatch.setattr(bridge, "client_listed", client_listed)
    old = bytes([9] * 32)
    entry = _entry(hass, DIRECT_DATA)
    store = kept(hass, entry.entry_id)
    await store.load()
    store.set_leftover_key(old)
    key_left(hass, fake.IDENTITY, old)
    feeder.push(fake.connected(), fake.update(bridge.UpdateKind.State, state=fake.state()))
    assert await hass.config_entries.async_setup(entry.entry_id)
    await _until(hass, lambda: len(asked) == 1)
    # Asked with the key in use now, about the one left over.
    assert asked[0] == (fake.SECRET, fake.IDENTITY, old, DIRECT_DATA[CONF_ADDRESS])
    assert _issue(hass, "leftover_key") is not None

    # Deleted while the feeder may still list it: the repair stays, worded
    # for no entry to check it.
    assert await hass.config_entries.async_remove(entry.entry_id)
    issue = _issue(hass, "leftover_key")
    assert issue is not None and issue.is_persistent
    assert issue.translation_key == "leftover_key_left"
    assert issue.translation_placeholders == {
        "feeder": fake.IDENTITY.hex()[:8],
        "key": old.hex()[:8],
    }


async def test_write_entities_are_hidden_while_the_role_writes_nothing(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder, SERVER_DATA)
    registry = er.async_get(hass)
    identity = fake.IDENTITY.hex()

    def hidden(domain: str, key: str) -> er.RegistryEntryHider | None:
        entity_id = registry.async_get_entity_id(domain, DOMAIN, f"{identity}-{key}")
        assert entity_id is not None
        found = registry.async_get(entity_id)
        assert found is not None
        return found.hidden_by

    writes = [("button", "feed"), ("switch", "child_lock"), ("number", "volume")]
    assert all(hidden(*entity) is None for entity in writes)

    # The account was demoted to a viewer while the session was away.
    feeder.push(fake.connected(role=bridge.Role.Viewer))
    integration = er.RegistryEntryHider.INTEGRATION
    await _until(hass, lambda: all(hidden(*entity) is integration for entity in writes))
    assert hidden("sensor", "food") is None
    feed = registry.async_get_entity_id("button", DOMAIN, f"{identity}-feed")
    assert feed is not None
    feed_state = hass.states.get(feed)
    assert feed_state is not None and feed_state.state == "unavailable"

    # A hide the user chose stays theirs.
    volume = registry.async_get_entity_id("number", DOMAIN, f"{identity}-volume")
    assert volume is not None
    registry.async_update_entity(volume, hidden_by=er.RegistryEntryHider.USER)
    feeder.push(fake.connected(role=bridge.Role.Owner))
    await _until(hass, lambda: hidden("button", "feed") is None)
    assert hidden("number", "volume") is er.RegistryEntryHider.USER
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_an_alert_that_opens_and_clears_fires_each_once(
    hass: HomeAssistant, feeder: fake.FakeFeeder
) -> None:
    entry = await _ready(hass, feeder)
    sensor = "sensor.feeder_00010203_alerts"
    event = "event.feeder_00010203_alert"
    food = fake.alert("4:food_low", bridge.AlertKind.FoodLow, bridge.AlertLevel.Soon)

    feeder.push(fake.alerts([]))
    await _until(hass, lambda: _state_of(hass, sensor) == "0")
    feeder.push(fake.alerts([food], opened=[food]))
    await _until(hass, lambda: _state_of(hass, sensor) == "1")
    opened = hass.states.get(event)
    assert opened is not None and opened.attributes["event_type"] == "opened"
    assert opened.attributes["kind"] == "food_low"
    assert opened.attributes["at"] is not None
    attributes = hass.states.get(sensor)
    assert attributes is not None and attributes.attributes["alerts"][0]["key"] == "4:food_low"

    feeder.push(fake.alerts([], cleared=[food]))
    await _until(hass, lambda: _state_of(hass, sensor) == "0")
    cleared = hass.states.get(event)
    assert cleared is not None and cleared.attributes["event_type"] == "cleared"
    assert cleared.attributes["key"] == "4:food_low"
    assert await hass.config_entries.async_unload(entry.entry_id)


def _state_of(hass: HomeAssistant, entity_id: str) -> str | None:
    state = hass.states.get(entity_id)
    return state.state if state is not None else None
