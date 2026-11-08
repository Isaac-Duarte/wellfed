"""Each feed as it ends: fed, cancelled, failed, uncertain or expired."""

from __future__ import annotations

from homeassistant.components.event import EventEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .entity import WellfedEntity, serves
from .sensor import feed_attributes
from .session import FeederSession

PARALLEL_UPDATES = 0

# A feed's outcome as an event type; a feed that ran to its end is `fed`.
# An outcome a newer feeder build added is uncertain here: it is never
# taken for a feed that ran.
TYPES = {
    bridge.FeedOutcome.Done.key: "fed",
    bridge.FeedOutcome.Cancelled.key: "cancelled",
    bridge.FeedOutcome.Failed.key: "failed",
    bridge.FeedOutcome.Uncertain.key: "uncertain",
    bridge.FeedOutcome.Expired.key: "expired",
    bridge.FeedOutcome.NewerBuild.key: "uncertain",
}


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    if serves(session, lambda caps: caps.feed):
        async_add_entities([FeedEvent(session, "feed_event")])


class FeedEvent(WellfedEntity, EventEntity):
    """A feed's end, with its portions and who asked. Only feeds as they
    end: the history read at connect is not news."""

    def __init__(self, session: FeederSession, key: str) -> None:
        super().__init__(session, key)
        self._attr_event_types = list(TYPES.values())

    async def async_added_to_hass(self) -> None:
        await super().async_added_to_hass()
        self.async_on_remove(self.session.add_event_listener(self._event))

    @callback
    def _event(self, event: bridge.Event) -> None:
        if event.fed is None:
            return
        self._trigger_event(TYPES[event.fed.outcome.key], feed_attributes(event.fed))
        self.async_write_ha_state()
