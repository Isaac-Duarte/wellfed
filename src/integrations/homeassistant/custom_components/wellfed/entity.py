"""The base every Wellfed entity shares: one device per feeder."""

from __future__ import annotations

from collections.abc import Callable

from homeassistant.core import callback
from homeassistant.helpers import entity_registry as er
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity import Entity

import wellfed_homeassistant as bridge

from .const import DOMAIN
from .session import FeederSession


class WellfedEntity(Entity):
    """An entity that follows its feeder's session."""

    _attr_has_entity_name = True
    _attr_should_poll = False
    # Whether it changes the feeder. Hidden while this client's role writes
    # nothing (a viewer's account, or a role a newer build added), and
    # shown again once it does: its unique id and history stay.
    _writes = False

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
        if self._writes and not self.session.writes:
            return False
        return self.session.connected and self.session.status is not None

    async def async_added_to_hass(self) -> None:
        self.async_on_remove(self.session.add_listener(self._changed))
        self._follow_role()

    @callback
    def _changed(self) -> None:
        self._follow_role()
        self.async_write_ha_state()

    @callback
    def _follow_role(self) -> None:
        """Hide a write entity while the role writes nothing, and show it again
        once it does; a hide the user chose is theirs."""
        entry = self.registry_entry
        if not self._writes or entry is None or self.session.role is None:
            return
        hider = er.RegistryEntryHider.INTEGRATION
        if self.session.writes and entry.hidden_by is hider:
            er.async_get(self.hass).async_update_entity(self.entity_id, hidden_by=None)
        elif not self.session.writes and entry.hidden_by is None:
            er.async_get(self.hass).async_update_entity(self.entity_id, hidden_by=hider)


def serves(session: FeederSession, capability: Callable[[bridge.Capabilities], bool]) -> bool:
    """Whether the feeder serves what `capability` names."""
    return session.capabilities is not None and capability(session.capabilities)
