"""Whether the session is open, food is low, the last feed failed, and the
feeder runs on its batteries."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from homeassistant.components.binary_sensor import (
    BinarySensorDeviceClass,
    BinarySensorEntity,
    BinarySensorEntityDescription,
)
from homeassistant.const import EntityCategory
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .entity import WellfedEntity
from .session import FeederSession

PARALLEL_UPDATES = 0


def _food_low(session: FeederSession) -> bool | None:
    status = session.status
    if status is None or status.food == bridge.FoodLevel.Unread:
        return None
    return status.food == bridge.FoodLevel.Low


def _feed_problem(session: FeederSession) -> bool | None:
    status = session.status
    return status.feed_fault is not None if status is not None else None


def _feed_problem_attributes(session: FeederSession) -> dict[str, Any]:
    status = session.status
    fault = status.feed_fault if status is not None else None
    if fault is None:
        return {}
    return {"feed": fault.feed, "outcome": fault.outcome.key, "reason": fault.reason.key}


def _on_battery(session: FeederSession) -> bool | None:
    status = session.status
    if status is None or status.adapter is None:
        return None
    return not status.adapter


@dataclass(frozen=True, kw_only=True)
class WellfedBinarySensorDescription(BinarySensorEntityDescription):
    value: Callable[[FeederSession], bool | None]
    attributes: Callable[[FeederSession], dict[str, Any]] = lambda _: {}


SENSORS = (
    WellfedBinarySensorDescription(
        key="food_low",
        translation_key="food_low",
        device_class=BinarySensorDeviceClass.PROBLEM,
        value=_food_low,
    ),
    WellfedBinarySensorDescription(
        key="feed_problem",
        translation_key="feed_problem",
        device_class=BinarySensorDeviceClass.PROBLEM,
        value=_feed_problem,
        attributes=_feed_problem_attributes,
    ),
    WellfedBinarySensorDescription(
        key="on_battery",
        translation_key="on_battery",
        value=_on_battery,
    ),
)


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    entities: list[WellfedEntity] = [Connected(session, "connected")]
    entities.extend(WellfedBinarySensor(session, description) for description in SENSORS)
    async_add_entities(entities)


class Connected(WellfedEntity, BinarySensorEntity):
    """The session itself, so it stays available while the feeder is not."""

    _attr_device_class = BinarySensorDeviceClass.CONNECTIVITY
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    @property
    def available(self) -> bool:
        return True

    @property
    def is_on(self) -> bool:
        return self.session.connected


class WellfedBinarySensor(WellfedEntity, BinarySensorEntity):
    entity_description: WellfedBinarySensorDescription

    def __init__(self, session: FeederSession, description: WellfedBinarySensorDescription) -> None:
        super().__init__(session, description.key)
        self.entity_description = description

    @property
    def is_on(self) -> bool | None:
        return self.entity_description.value(self.session)

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        return self.entity_description.attributes(self.session)
