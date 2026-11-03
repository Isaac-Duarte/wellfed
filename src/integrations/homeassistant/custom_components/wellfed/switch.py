"""The panel's child lock."""

from __future__ import annotations

from typing import Any

from homeassistant.components.switch import SwitchEntity
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
    async_add_entities([LockSwitch(entry.runtime_data, "child_lock")])


class LockSwitch(WellfedEntity, SwitchEntity):
    """On while the panel is locked. Its state is the feeder's, never assumed."""

    _attr_entity_category = EntityCategory.CONFIG

    @property
    def is_on(self) -> bool | None:
        status = self.session.status
        return None if status is None else status.locked

    async def async_turn_on(self, **kwargs: Any) -> None:
        await self.session.set_lock(True)

    async def async_turn_off(self, **kwargs: Any) -> None:
        await self.session.set_lock(False)
