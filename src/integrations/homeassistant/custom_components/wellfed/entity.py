"""The base every Wellfed entity shares: one device per feeder."""

from __future__ import annotations

from collections.abc import Callable

from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity import Entity

import wellfed_homeassistant as bridge

from .const import DOMAIN
from .session import FeederSession


class WellfedEntity(Entity):
    """An entity that follows its feeder's session."""

    _attr_has_entity_name = True
    _attr_should_poll = False

    def __init__(self, session: FeederSession, key: str) -> None:
        self.session = session
        identity = session.identity.hex()
        self._attr_translation_key = key
        self._attr_unique_id = f"{identity}-{key}"
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, identity)},
            manufacturer="Wellfed",
            model="PLAF103",
            name=f"Feeder {identity[:8]}",
            sw_version=session.build,
        )

    @property
    def available(self) -> bool:
        return self.session.connected and self.session.status is not None

    async def async_added_to_hass(self) -> None:
        self.async_on_remove(self.session.add_listener(self.async_write_ha_state))


def serves(session: FeederSession, capability: Callable[[bridge.Capabilities], bool]) -> bool:
    """Whether the feeder serves what `capability` names."""
    return session.capabilities is not None and capability(session.capabilities)
