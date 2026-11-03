"""Each entity end to end against a simulated feeder: its state is the
feeder's, and each control reaches the feeder and comes back as state."""

from __future__ import annotations

import asyncio
from collections.abc import Callable

from homeassistant.config_entries import SOURCE_USER, ConfigEntry
from homeassistant.const import ATTR_ENTITY_ID, STATE_OFF, STATE_ON, STATE_UNKNOWN
from homeassistant.core import HomeAssistant, State
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.exceptions import HomeAssistantError, ServiceValidationError
from homeassistant.helpers import device_registry as dr, entity_registry as er
import pytest

from custom_components.wellfed.const import CONF_ADDRESS, CONF_INVITE, DOMAIN

from .conftest import Simulator

WAIT = 60


async def _until(hass: HomeAssistant, condition: Callable[[], bool]) -> None:
    async with asyncio.timeout(WAIT):
        while not condition():
            await asyncio.sleep(0.1)
            await hass.async_block_till_done()


async def _pair(hass: HomeAssistant, simulator: Simulator, role: str = "member") -> ConfigEntry:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"],
        {CONF_INVITE: simulator.invite(role), CONF_ADDRESS: simulator.address},
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    entry: ConfigEntry = result["result"]
    await hass.async_block_till_done()
    return entry


def _id(entry: ConfigEntry, domain: str, name: str) -> str:
    assert entry.unique_id is not None
    return f"{domain}.feeder_{entry.unique_id[:8]}_{name}"


def _state(hass: HomeAssistant, entity_id: str) -> State:
    state = hass.states.get(entity_id)
    assert state is not None, entity_id
    return state


async def _press(hass: HomeAssistant, entity_id: str) -> None:
    await hass.services.async_call("button", "press", {ATTR_ENTITY_ID: entity_id}, blocking=True)


async def _set(hass: HomeAssistant, entity_id: str, value: float) -> None:
    await hass.services.async_call(
        "number", "set_value", {ATTR_ENTITY_ID: entity_id, "value": value}, blocking=True
    )


async def _switch(hass: HomeAssistant, entity_id: str, on: bool) -> None:
    service = "turn_on" if on else "turn_off"
    await hass.services.async_call("switch", service, {ATTR_ENTITY_ID: entity_id}, blocking=True)


async def test_a_member_gets_the_member_entities_and_no_owner_ones(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    registry = er.async_get(hass)
    ids = {item.entity_id for item in er.async_entries_for_config_entry(registry, entry.entry_id)}

    for domain, name in [
        ("button", "feed"),
        ("button", "cancel_feed"),
        ("button", "desiccant_replaced"),
        ("button", "play_chime"),
        ("number", "portions"),
        ("number", "volume"),
        ("switch", "child_lock"),
        ("switch", "sound"),
        ("switch", "lights"),
        ("sensor", "last_fed"),
        ("sensor", "portions_today"),
        ("sensor", "next_meal"),
        ("sensor", "food"),
        ("sensor", "readiness"),
        ("sensor", "battery"),
        ("sensor", "signal_strength"),
        ("sensor", "desiccant_days_left"),
        ("binary_sensor", "connectivity"),
        ("binary_sensor", "food_low"),
        ("binary_sensor", "feed_problem"),
        ("binary_sensor", "on_battery"),
        ("event", "feed"),
    ]:
        assert _id(entry, domain, name) in ids, (domain, name)
    for domain, name in [("button", "restart"), ("number", "portion_weight")]:
        assert _id(entry, domain, name) not in ids, (domain, name)
    assert not any(entity_id.startswith("update.") for entity_id in ids)

    signal = registry.async_get(_id(entry, "sensor", "signal_strength"))
    assert signal is not None and signal.disabled_by is not None, "off by default"
    assert _state(hass, _id(entry, "sensor", "readiness")).state == "ready"
    assert _state(hass, _id(entry, "sensor", "food")).state in ("unread", "present")
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_feed_button_asks_for_the_portions_number_and_the_event_fires(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    # A feed is dated by the feeder's clock, which the owner sets.
    simulator.owner("time")
    entry = await _pair(hass, simulator)
    portions = _id(entry, "number", "portions")
    event = _id(entry, "event", "feed")

    await _set(hass, portions, 2)
    assert _state(hass, portions).state == "2"
    await _press(hass, _id(entry, "button", "feed"))

    await _until(hass, lambda: _state(hass, event).attributes.get("event_type") == "fed")
    fired = _state(hass, event).attributes
    assert (fired["requested"], fired["completed"], fired["by"]) == (2, 2, "client")

    last_fed = _state(hass, _id(entry, "sensor", "last_fed"))
    await _until(
        hass,
        lambda: _state(hass, _id(entry, "sensor", "last_fed")).attributes.get("feed")
        == fired["feed"],
    )
    assert last_fed.state != STATE_UNKNOWN
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_cancel_button_stops_the_feed_home_assistant_started(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    cancel = _id(entry, "button", "cancel_feed")
    assert _state(hass, cancel).state == "unavailable", "nothing to cancel"

    await _set(hass, _id(entry, "number", "portions"), 10)
    await _press(hass, _id(entry, "button", "feed"))
    await _until(hass, lambda: _state(hass, cancel).state != "unavailable")
    await _press(hass, cancel)

    event = _id(entry, "event", "feed")
    await _until(hass, lambda: _state(hass, event).attributes.get("event_type") == "cancelled")
    assert _state(hass, event).attributes["completed"] < 10
    await _until(hass, lambda: _state(hass, cancel).state == "unavailable")

    device = dr.async_get(hass).async_get_device_by_identifier(
        (DOMAIN, entry.unique_id or ""), entry.entry_id
    )
    assert device is not None
    with pytest.raises(ServiceValidationError):
        await hass.services.async_call(
            DOMAIN, "cancel_feed", {"device_id": device.id}, blocking=True
        )
    assert await hass.config_entries.async_unload(entry.entry_id)


def _is(hass: HomeAssistant, entity_id: str, state: str) -> Callable[[], bool]:
    return lambda: _state(hass, entity_id).state == state


async def test_settings_switches_and_the_volume_follow_the_feeder(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    volume = _id(entry, "number", "volume")
    await _set(hass, volume, 35)
    await _until(hass, lambda: _state(hass, volume).state == "35")

    for name in ("sound", "lights", "child_lock"):
        entity_id = _id(entry, "switch", name)
        start = _state(hass, entity_id).state == STATE_ON
        await _switch(hass, entity_id, not start)
        flipped = STATE_OFF if start else STATE_ON
        await _until(hass, _is(hass, entity_id, flipped))
        await _switch(hass, entity_id, start)
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_plan_switches_follow_the_plan_table(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    simulator.owner("time")
    entry = await _pair(hass, simulator)
    registry = er.async_get(hass)
    plan = _id(entry, "switch", "meal_07_30")

    simulator.owner("plans", "add", "--at", "07:30", "--portions", "2")
    await _until(hass, lambda: hass.states.get(plan) is not None)
    assert _state(hass, plan).state == STATE_ON
    await _until(
        hass, lambda: _state(hass, _id(entry, "sensor", "next_meal")).state != STATE_UNKNOWN
    )
    assert _state(hass, _id(entry, "sensor", "next_meal")).attributes["portions"] == 2

    await _switch(hass, plan, False)
    await _until(hass, lambda: _state(hass, plan).state == STATE_OFF)
    assert "disabled" in simulator.owner("plans", "list")

    simulator.owner("plans", "delete", "0")
    await _until(hass, lambda: registry.async_get(plan) is None)
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_desiccant_is_reset_once_the_feeder_has_a_clock(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    simulator.owner("time")
    entry = await _pair(hass, simulator)
    days = _id(entry, "sensor", "desiccant_days_left")
    assert _state(hass, days).state == STATE_UNKNOWN, "never replaced"

    await _press(hass, _id(entry, "button", "desiccant_replaced"))
    await _until(hass, lambda: _state(hass, days).state not in (STATE_UNKNOWN, "unavailable"))
    assert int(_state(hass, days).state) > 0
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_the_binary_sensors_follow_the_hopper_the_adapter_and_a_fault(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    on_battery = _id(entry, "binary_sensor", "on_battery")
    food_low = _id(entry, "binary_sensor", "food_low")
    problem = _id(entry, "binary_sensor", "feed_problem")

    simulator.console("adapter out")
    await _until(hass, lambda: _state(hass, on_battery).state == STATE_ON)
    simulator.console("adapter in")
    await _until(hass, lambda: _state(hass, on_battery).state == STATE_OFF)

    simulator.console("empty")
    await _press(hass, _id(entry, "button", "feed"))
    await _until(hass, lambda: _state(hass, food_low).state == STATE_ON)
    simulator.console("fill")

    assert _state(hass, problem).state == STATE_OFF
    simulator.console("jam")
    await _until(hass, lambda: _state(hass, _id(entry, "sensor", "readiness")).state == "ready")
    await _press(hass, _id(entry, "button", "feed"))
    await _until(hass, lambda: _state(hass, problem).state == STATE_ON)
    assert _state(hass, problem).attributes["outcome"] == "failed"
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_an_owner_gets_the_owner_entities(hass: HomeAssistant, simulator: Simulator) -> None:
    entry = await _pair(hass, simulator, role="owner")
    weight = _id(entry, "number", "portion_weight")
    assert hass.states.get(_id(entry, "button", "restart")) is not None
    firmware = _state(hass, _id(entry, "update", "firmware"))
    assert firmware.state == STATE_OFF, "nothing newer held"

    await _set(hass, weight, 9.5)
    await _until(hass, lambda: _state(hass, weight).state == "9.5")

    device = dr.async_get(hass).async_get_device_by_identifier(
        (DOMAIN, entry.unique_id or ""), entry.entry_id
    )
    assert device is not None
    with pytest.raises(HomeAssistantError):
        await hass.services.async_call(
            DOMAIN,
            "add_release",
            {"device_id": device.id, "path": "/not/allowed.plota"},
            blocking=True,
        )
    assert await hass.config_entries.async_unload(entry.entry_id)
