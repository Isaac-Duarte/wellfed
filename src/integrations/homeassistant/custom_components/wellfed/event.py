"""Each feed as it ends (fed, cancelled, failed, uncertain or expired), and
each alert as it opens or clears."""

from __future__ import annotations

from homeassistant.components.event import EventEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .entity import WellfedEntity, serves
from .sensor import alert_attributes, feed_attributes
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
    entities: list[EventEntity] = [AlertEvent(session, "alert_event")]
    if serves(session, lambda caps: caps.feed):
        entities.append(FeedEvent(session, "feed_event"))
    async_add_entities(entities)


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


class AlertEvent(WellfedEntity, EventEntity):
    """Each of the feeder's alerts as it opens or clears, with its key, kind
    and level. The feeder decides every alert; what is open when Home
    Assistant starts shows in the alerts sensor and fires nothing."""

    def __init__(self, session: FeederSession, key: str) -> None:
        super().__init__(session, key)
        self._attr_event_types = ["opened", "cleared"]

    async def async_added_to_hass(self) -> None:
        await super().async_added_to_hass()
        self.async_on_remove(self.session.add_alert_listener(self._alerts))

    @callback
    def _alerts(self, alerts: bridge.Alerts) -> None:
        for event_type, changed in (("opened", alerts.opened), ("cleared", alerts.cleared)):
            for alert in changed:
                self._trigger_event(event_type, alert_attributes(alert))
                self.async_write_ha_state()
