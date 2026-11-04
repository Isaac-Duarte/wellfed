"""The feeder's day and its readings: the last feed, portions today, the
next meal, food, readiness, battery, signal and desiccant."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from homeassistant.components.sensor import (
    SensorDeviceClass,
    SensorEntity,
    SensorEntityDescription,
    SensorStateClass,
)
from homeassistant.const import (
    PERCENTAGE,
    SIGNAL_STRENGTH_DECIBELS_MILLIWATT,
    EntityCategory,
    UnitOfTime,
)
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.util import dt as dt_util

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .entity import WellfedEntity, serves
from .session import FeederSession

PARALLEL_UPDATES = 0

FOOD_LEVELS = [
    level.key for level in (bridge.FoodLevel.Unread, bridge.FoodLevel.Present, bridge.FoodLevel.Low)
]
READINESS = [
    readiness.key
    for readiness in (
        bridge.Readiness.Ready,
        bridge.Readiness.Starting,
        bridge.Readiness.Journal,
        bridge.Readiness.Fault,
        bridge.Readiness.Resolving,
        bridge.Readiness.Feeding,
        bridge.Readiness.Joining,
        bridge.Readiness.Updating,
        bridge.Readiness.Setup,
        bridge.Readiness.Unserved,
        bridge.Readiness.Battery,
        bridge.Readiness.Restarting,
    )
]


def at(unix_ms: int | None) -> datetime | None:
    """A feeder time, Unix milliseconds, as an aware datetime."""
    return dt_util.utc_from_timestamp(unix_ms / 1000) if unix_ms is not None else None


def feed_attributes(fed: bridge.Feed) -> dict[str, Any]:
    """A feed's facts. An uncertain feed's portions are a range: at least
    `completed`, at most `at_most`."""
    attributes: dict[str, Any] = {
        "feed": fed.feed,
        "requested": fed.requested,
        "completed": fed.completed,
        "at_most": fed.at_most,
        "outcome": fed.outcome.key,
        "reason": fed.reason.key,
        "by": fed.by.key if fed.by is not None else None,
    }
    if fed.client is not None:
        attributes["client"] = fed.client
    if fed.plan is not None:
        attributes["plan"] = fed.plan
    return attributes


def _last_fed(session: FeederSession) -> datetime | None:
    fed = session.last_fed
    return at(fed.at_ms) if fed is not None else None


def _portions_today(session: FeederSession) -> int | None:
    state = session.state
    if state is None or state.portions_today is None:
        return None
    return state.portions_today.confirmed


def _portions_today_attributes(session: FeederSession) -> dict[str, Any]:
    state = session.state
    if state is None or state.portions_today is None:
        return {}
    today = state.portions_today
    return {"at_most": today.at_most, "feeds": today.feeds}


def _next_meal(session: FeederSession) -> datetime | None:
    state = session.state
    if state is None or state.next_meal is None:
        return None
    return at(state.next_meal.at_ms)


def _next_meal_attributes(session: FeederSession) -> dict[str, Any]:
    state = session.state
    if state is None or state.next_meal is None:
        return {}
    meal = state.next_meal
    return {"plan": meal.slot, "portions": meal.portions}


def _readiness(session: FeederSession) -> str | None:
    status = session.status
    if status is None:
        return None
    return status.readiness.key


@dataclass(frozen=True, kw_only=True)
class WellfedSensorDescription(SensorEntityDescription):
    value: Callable[[FeederSession], Any]
    attributes: Callable[[FeederSession], dict[str, Any]] = lambda _: {}
    offered: Callable[[FeederSession], bool] = lambda _: True


SENSORS = (
    WellfedSensorDescription(
        key="last_fed",
        translation_key="last_fed",
        device_class=SensorDeviceClass.TIMESTAMP,
        value=_last_fed,
        attributes=lambda session: (
            feed_attributes(session.last_fed) if session.last_fed is not None else {}
        ),
        offered=lambda session: serves(session, lambda caps: caps.feed),
    ),
    WellfedSensorDescription(
        key="portions_today",
        translation_key="portions_today",
        state_class=SensorStateClass.TOTAL,
        value=_portions_today,
        attributes=_portions_today_attributes,
        offered=lambda session: serves(session, lambda caps: caps.feed),
    ),
    WellfedSensorDescription(
        key="next_meal",
        translation_key="next_meal",
        device_class=SensorDeviceClass.TIMESTAMP,
        value=_next_meal,
        attributes=_next_meal_attributes,
        offered=lambda session: serves(session, lambda caps: caps.plans),
    ),
    WellfedSensorDescription(
        key="food",
        translation_key="food",
        device_class=SensorDeviceClass.ENUM,
        options=FOOD_LEVELS,
        value=lambda session: session.status.food.key if session.status is not None else None,
    ),
    WellfedSensorDescription(
        key="readiness",
        translation_key="readiness",
        device_class=SensorDeviceClass.ENUM,
        options=READINESS,
        value=_readiness,
    ),
    WellfedSensorDescription(
        key="battery",
        device_class=SensorDeviceClass.BATTERY,
        native_unit_of_measurement=PERCENTAGE,
        state_class=SensorStateClass.MEASUREMENT,
        value=lambda session: (
            session.status.battery_percent if session.status is not None else None
        ),
    ),
    WellfedSensorDescription(
        key="rssi",
        device_class=SensorDeviceClass.SIGNAL_STRENGTH,
        native_unit_of_measurement=SIGNAL_STRENGTH_DECIBELS_MILLIWATT,
        state_class=SensorStateClass.MEASUREMENT,
        entity_category=EntityCategory.DIAGNOSTIC,
        entity_registry_enabled_default=False,
        value=lambda session: session.status.rssi if session.status is not None else None,
    ),
    WellfedSensorDescription(
        key="desiccant",
        translation_key="desiccant",
        native_unit_of_measurement=UnitOfTime.DAYS,
        value=lambda session: (
            session.status.desiccant_days if session.status is not None else None
        ),
        offered=lambda session: serves(session, lambda caps: caps.desiccant),
    ),
)


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    async_add_entities(
        WellfedSensor(session, description)
        for description in SENSORS
        if description.offered(session)
    )


class WellfedSensor(WellfedEntity, SensorEntity):
    entity_description: WellfedSensorDescription

    def __init__(self, session: FeederSession, description: WellfedSensorDescription) -> None:
        super().__init__(session, description.key)
        self.entity_description = description
        self._attr_translation_key = description.translation_key

    @property
    def native_value(self) -> Any:
        return self.entity_description.value(self.session)

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        return self.entity_description.attributes(self.session)
