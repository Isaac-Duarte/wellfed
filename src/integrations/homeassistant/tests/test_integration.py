"""The integration against a simulated feeder: pairing, entities and feeds."""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Mapping
from typing import Any

from homeassistant.config_entries import SOURCE_USER, ConfigEntry
from homeassistant.const import ATTR_ENTITY_ID, STATE_OFF, STATE_ON
from homeassistant.core import HomeAssistant, State
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.helpers import device_registry as dr
import pytest

from custom_components.wellfed.const import (
    CONF_ADDRESS,
    CONF_INVITE,
    DOMAIN,
    EVENT_FEEDER,
)

from .conftest import Simulator

WAIT = 30


async def _until(hass: HomeAssistant, condition: Callable[[], bool]) -> None:
    async with asyncio.timeout(WAIT):
        while not condition():
            await asyncio.sleep(0.1)
            await hass.async_block_till_done()


async def _pair(hass: HomeAssistant, simulator: Simulator) -> ConfigEntry:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"],
        {CONF_INVITE: simulator.invite(), CONF_ADDRESS: simulator.address},
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    await hass.async_block_till_done()
    entry = result["result"]
    await _until(hass, lambda: entry.runtime_data.status is not None)
    return entry


async def test_a_text_that_is_not_an_invite_is_refused(hass: HomeAssistant) -> None:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_INVITE: "not an invite"}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {CONF_INVITE: "invalid_invite"}


async def test_a_spent_invite_asks_for_a_new_one(hass: HomeAssistant, simulator: Simulator) -> None:
    invite = simulator.invite()
    subprocess_join = (
        simulator.cli,
        "join",
        "--create-key",
        "--keys",
        simulator.keys.with_name("other.json"),
    )
    proc = await asyncio.create_subprocess_exec(
        *subprocess_join, "--address", simulator.address, invite
    )
    assert await proc.wait() == 0

    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_INVITE: invite, CONF_ADDRESS: simulator.address}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"base": "join_failed"}


async def test_the_button_feeds_once_to_its_fed_event(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    fed: list[Mapping[str, Any]] = []
    hass.bus.async_listen(
        EVENT_FEEDER,
        lambda event: fed.append(event.data) if event.data["kind"] == "fed" else None,
    )

    await hass.services.async_call(
        "button",
        "press",
        {ATTR_ENTITY_ID: "button.feeder_" + _short(entry) + "_feed"},
        blocking=True,
    )
    await _until(hass, lambda: len(fed) == 1)
    assert fed[0]["completed"] == 1
    await _until(
        hass,
        lambda: _state(hass, "sensor.feeder_" + _short(entry) + "_last_fed").attributes.get("feed")
        == fed[0]["feed"],
    )

    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_feed_action_answers_the_feeders_number(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    device = _device(hass, entry)

    response = await hass.services.async_call(
        DOMAIN,
        "feed",
        {"device_id": device.id, "portions": 2},
        blocking=True,
        return_response=True,
    )
    assert response is not None
    assert isinstance(response["feed"], int)

    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_lock_switch_follows_the_feeder(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    lock = "switch.feeder_" + _short(entry) + "_child_lock"

    await hass.services.async_call("switch", "turn_on", {ATTR_ENTITY_ID: lock}, blocking=True)
    await _until(hass, lambda: _state(hass, lock).state == STATE_ON)
    await hass.services.async_call("switch", "turn_off", {ATTR_ENTITY_ID: lock}, blocking=True)
    await _until(hass, lambda: _state(hass, lock).state == STATE_OFF)

    assert await hass.config_entries.async_unload(entry.entry_id)


@pytest.mark.parametrize("portions", [0, 49])
async def test_the_feed_action_refuses_portions_out_of_range(
    hass: HomeAssistant, simulator: Simulator, portions: int
) -> None:
    entry = await _pair(hass, simulator)
    device = _device(hass, entry)

    with pytest.raises(Exception, match="value must be"):
        await hass.services.async_call(
            DOMAIN, "feed", {"device_id": device.id, "portions": portions}, blocking=True
        )

    assert await hass.config_entries.async_unload(entry.entry_id)


def _short(entry: ConfigEntry) -> str:
    assert entry.unique_id is not None
    return entry.unique_id[:8]


def _state(hass: HomeAssistant, entity_id: str) -> State:
    state = hass.states.get(entity_id)
    assert state is not None, entity_id
    return state


def _device(hass: HomeAssistant, entry: ConfigEntry) -> dr.DeviceEntry:
    assert entry.unique_id is not None
    device = dr.async_get(hass).async_get_device_by_identifier(
        (DOMAIN, entry.unique_id), entry.entry_id
    )
    assert device is not None
    return device
