"""The feed button: one press, one feed of one portion."""

from __future__ import annotations

from homeassistant.components.button import ButtonEntity
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from . import WellfedConfigEntry
from .entity import WellfedEntity


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([FeedButton(entry.runtime_data, "feed")])


class FeedButton(WellfedEntity, ButtonEntity):
    """Each press sends one feed request, under its own request id."""

    async def async_press(self) -> None:
        await self.session.feed(1)
