"""The feeder's last feed, signal, battery and desiccant."""

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

from . import WellfedConfigEntry
from .entity import WellfedEntity
from .session import FeederSession


def _battery(session: FeederSession) -> int | None:
    return session.status.battery_percent if session.status is not None else None


def _desiccant(session: FeederSession) -> int | None:
    return session.status.desiccant_days if session.status is not None else None


def _last_fed(session: FeederSession) -> datetime | None:
    fed = session.last_fed
    if fed is None or fed.at_ms is None:
        return None
    return dt_util.utc_from_timestamp(fed.at_ms / 1000)


def _last_fed_attributes(session: FeederSession) -> dict[str, Any]:
    fed = session.last_fed
    if fed is None:
        return {}
    attributes: dict[str, Any] = {
        "feed": fed.feed,
        "requested": fed.requested,
        "completed": fed.completed,
        "outcome": fed.outcome.key,
        "reason": fed.reason.key,
        "by": fed.by.key,
    }
    if fed.client is not None:
        attributes["client"] = fed.client
    if fed.plan is not None:
        attributes["plan"] = fed.plan
    return attributes


@dataclass(frozen=True, kw_only=True)
class WellfedSensorDescription(SensorEntityDescription):
    value: Callable[[FeederSession], Any]
    attributes: Callable[[FeederSession], dict[str, Any]] = lambda _: {}


SENSORS = (
    WellfedSensorDescription(
        key="last_fed",
        translation_key="last_fed",
        device_class=SensorDeviceClass.TIMESTAMP,
        value=_last_fed,
        attributes=_last_fed_attributes,
    ),
    WellfedSensorDescription(
        key="battery",
        device_class=SensorDeviceClass.BATTERY,
        native_unit_of_measurement=PERCENTAGE,
        state_class=SensorStateClass.MEASUREMENT,
        value=_battery,
    ),
    WellfedSensorDescription(
        key="desiccant",
        translation_key="desiccant",
        native_unit_of_measurement=UnitOfTime.DAYS,
        value=_desiccant,
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
)


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities(WellfedSensor(entry.runtime_data, description) for description in SENSORS)


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
