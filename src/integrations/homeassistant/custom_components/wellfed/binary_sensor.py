"""Whether the session is open, and whether food is low."""

from __future__ import annotations

from homeassistant.components.binary_sensor import (
    BinarySensorDeviceClass,
    BinarySensorEntity,
)
from homeassistant.const import EntityCategory
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from . import WellfedConfigEntry
from .entity import WellfedEntity


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    async_add_entities([Connected(session, "connected"), FoodLow(session, "food_low")])


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


class FoodLow(WellfedEntity, BinarySensorEntity):
    """The food beam's last reading; unknown until the beam has read."""

    _attr_device_class = BinarySensorDeviceClass.PROBLEM

    @property
    def is_on(self) -> bool | None:
        status = self.session.status or {}
        food = status.get("food")
        if food == "unread":
            return None
        return food == "low"
