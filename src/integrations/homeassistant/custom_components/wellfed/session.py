"""One feeder's session, as Home Assistant sees it.

The Rust bridge owns the connection, the keepalive and the event cursor on
a thread of its own. This class pumps the bridge's updates into the event
loop and hands writes to the executor. It holds no feeding logic: a write
goes out once through the bridge, and a write whose answer never came is
reported as uncertain and never sent again.
"""

from __future__ import annotations

import asyncio
from collections import deque
from collections.abc import Callable
import contextlib
import logging
import threading

from homeassistant.core import HomeAssistant, callback
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import issue_registry as ir
from homeassistant.util import dt as dt_util

import wellfed_homeassistant as bridge

from .const import DOMAIN, EVENT_FEEDER

_LOGGER = logging.getLogger(__name__)

# How long the pump waits on the bridge before it looks for a stop.
PUMP_WAIT = 1.0

# Events kept for diagnostics.
RECENT_EVENTS = 20

# A feed whose answer was lost, until its `Fed` event settles it.
UNCERTAIN_ISSUE = "uncertain_feed"
# A lost feed the feeder's history holds no record of.
UNKNOWN_ISSUE = "unknown_feed"


class WriteNotDone(HomeAssistantError):
    """A write the feeder did not carry out, or may have, in words the user reads."""

    def __init__(self, key: str) -> None:
        super().__init__(translation_domain=DOMAIN, translation_key=key)


def _datetime(unix_ms: int | None) -> str | None:
    """A feeder time as ISO 8601, UTC."""
    if unix_ms is None:
        return None
    return dt_util.utc_from_timestamp(unix_ms / 1000).isoformat()


def event_data(identity: bytes, event: bridge.Event) -> dict[str, object]:
    """An event as the bus carries it: plain values an automation can match."""
    data: dict[str, object] = {
        "identity": identity.hex(),
        "seq": event.seq,
        "at": _datetime(event.at_ms),
        "kind": event.kind.key,
    }
    if event.by is not None:
        data["by"] = event.by.key
    if event.client is not None:
        data["client"] = event.client
    if event.plan is not None:
        data["plan"] = event.plan
    if event.locked is not None:
        data["locked"] = event.locked
    if event.fed is not None:
        fed = event.fed
        data.update(
            feed=fed.feed,
            requested=fed.requested,
            completed=fed.completed,
            outcome=fed.outcome.key,
            reason=fed.reason.key,
            hopper=fed.hopper.key,
        )
    return data


class FeederSession:
    """The bridge's session, what it knows of the feeder, and its listeners."""

    def __init__(
        self,
        hass: HomeAssistant,
        secret: bytes,
        identity: bytes,
        address: str | None,
    ) -> None:
        self.hass = hass
        self.identity = identity
        self._feeder: bridge.Feeder | None = bridge.Feeder(secret, identity, address)
        self._stop = threading.Event()
        self._pump: threading.Thread | None = None
        self._listeners: list[Callable[[], None]] = []

        self.connected = False
        self.revoked = False
        # Called once the feeder no longer knows this Home Assistant.
        self.on_revoked: Callable[[], None] | None = None
        self.build: str | None = None
        self.role: bridge.Role | None = None
        self.capabilities: bridge.Capabilities | None = None
        self.state: bridge.State | None = None
        # The feed this Home Assistant started, until its `Fed` event.
        self.running_feed: int | None = None
        # The Portions number's value: what the Feed button asks for.
        self.portions = 1
        self._event_listeners: list[Callable[[bridge.Event], None]] = []
        self._ready = asyncio.Event()
        # Feeds whose answer was lost, until each is settled or found unknown.
        self._uncertain = 0
        # The last events as they happened, for diagnostics.
        self.recent: deque[dict[str, object]] = deque(maxlen=RECENT_EVENTS)

    @property
    def status(self) -> bridge.Status | None:
        """The feeder's status as last read."""
        return self.state.status if self.state is not None else None

    @property
    def last_fed(self) -> bridge.Feed | None:
        """The newest feed the feeder's history has given."""
        return self.state.last_fed if self.state is not None else None

    def start(self) -> None:
        """Start pumping updates into the event loop."""
        self._pump = threading.Thread(target=self._run, name="wellfed-pump", daemon=True)
        self._pump.start()

    def stop(self) -> None:
        """Stop the pump; dropping the bridge's handle ends its session."""
        self._stop.set()
        if self._pump is not None:
            self._pump.join(timeout=PUMP_WAIT * 3)
        self._feeder = None

    async def ready(self, timeout: float) -> None:
        """Wait until the first state is in, or the key is found revoked."""
        with contextlib.suppress(TimeoutError):
            async with asyncio.timeout(timeout):
                await self._ready.wait()

    @callback
    def add_event_listener(self, listener: Callable[[bridge.Event], None]) -> Callable[[], None]:
        """Call `listener` with each event as it happens (never a replay)."""
        self._event_listeners.append(listener)
        return lambda: self._event_listeners.remove(listener)

    @callback
    def add_listener(self, listener: Callable[[], None]) -> Callable[[], None]:
        """Call `listener` on every change; returns its removal."""
        self._listeners.append(listener)
        return lambda: self._listeners.remove(listener)

    async def feed(self, portions: int) -> int:
        """Start a feed; the feeder's number for it."""
        try:
            written = await self._write(lambda feeder: feeder.feed(portions))
        except WriteNotDone as error:
            if error.translation_key == "write_uncertain":
                self._uncertain += 1
                self._issue(UNCERTAIN_ISSUE, ir.IssueSeverity.WARNING, {})
            raise
        if written.feed is None:
            raise WriteNotDone("write_uncertain")
        self.running_feed = written.feed
        self._changed()
        return written.feed

    async def cancel_feed(self, feed: int) -> None:
        """Stop the feed the feeder numbered `feed`, if it still runs."""
        await self._write(lambda feeder: feeder.cancel_feed(feed))

    async def set_lock(self, locked: bool) -> None:
        """Lock or unlock the panel."""
        await self._write(lambda feeder: feeder.set_lock(locked))

    async def reset_desiccant(self) -> None:
        """Record a fresh desiccant pack today."""
        await self._write(lambda feeder: feeder.reset_desiccant())

    async def play(self, tune: bridge.Tune) -> None:
        """Play one of the feeder's tunes."""
        await self._write(lambda feeder: feeder.play(tune))

    async def put_settings(
        self,
        *,
        sound: bool | None = None,
        lights: bool | None = None,
        volume: int | None = None,
        desiccant_days: int | None = None,
    ) -> None:
        """Change the settings named, on the settings last read."""
        await self._write(
            lambda feeder: feeder.put_settings(
                sound=sound, lights=lights, volume=volume, desiccant_days=desiccant_days
            )
        )

    async def set_plan_enabled(self, slot: int, enabled: bool) -> None:
        """Turn the plan in `slot` on or off, on the plans last read."""
        await self._write(lambda feeder: feeder.set_plan_enabled(slot, enabled))

    async def set_portion_weight(self, mg: int) -> None:
        """Store the weight of one portion."""
        await self._write(lambda feeder: feeder.set_portion_weight(mg))

    async def _write(self, call: Callable[[bridge.Feeder], bridge.Written]) -> bridge.Written:
        feeder = self._feeder
        if feeder is None:
            raise WriteNotDone("not_sent_stopped")
        written = await self.hass.async_add_executor_job(call, feeder)
        result = written.result
        if result in (bridge.WriteResult.Feeding, bridge.WriteResult.Done):
            return written
        if result == bridge.WriteResult.Uncertain:
            # It may have run. The feeder's event settles it; sending it
            # again could feed twice.
            raise WriteNotDone("write_uncertain")
        if result == bridge.WriteResult.NotSent:
            reason = written.not_sent or bridge.NotSent.Disconnected
            raise WriteNotDone(f"not_sent_{reason.key}")
        refusal = written.refusal or bridge.RefusalKind.Invalid
        raise WriteNotDone(f"refused_{refusal.key}")

    def _run(self) -> None:
        while not self._stop.is_set():
            feeder = self._feeder
            if feeder is None:
                return
            update = feeder.next_update(PUMP_WAIT)
            if update is not None:
                self.hass.loop.call_soon_threadsafe(self._apply, update)

    @callback
    def _apply(self, update: bridge.Update) -> None:
        kind = update.kind
        if kind == bridge.UpdateKind.Connected:
            self.connected = True
            self.build = update.build
            self.role = update.role
            self.capabilities = update.capabilities
        elif kind == bridge.UpdateKind.State:
            self.state = update.state
            self._ready.set()
        elif kind == bridge.UpdateKind.Event and update.event is not None:
            # A replayed event is history the state already holds, not news.
            event = update.event
            if event.fed is not None and event.fed.feed == self.running_feed:
                self.running_feed = None
            if not update.replayed:
                data = event_data(self.identity, event)
                self.recent.append(data)
                self.hass.bus.async_fire(EVENT_FEEDER, data)
                for hear in list(self._event_listeners):
                    hear(event)
        elif kind == bridge.UpdateKind.Gap:
            _LOGGER.info(
                "Some feeder events were not received: %s to %s", update.gap_from, update.gap_to
            )
        elif kind == bridge.UpdateKind.Settled and update.feed is not None:
            _LOGGER.info(
                "The feed whose answer was lost ran: feed %s, %s of %s portions",
                update.feed.feed,
                update.feed.completed,
                update.feed.requested,
            )
            self._settled()
        elif kind == bridge.UpdateKind.StillUnknown:
            _LOGGER.warning(
                "A feed of %s portions whose answer was lost left no record: whether it "
                "ran is not known",
                update.portions,
            )
            self._settled()
            self._issue(UNKNOWN_ISSUE, ir.IssueSeverity.WARNING, {"portions": str(update.portions)})
        elif kind == bridge.UpdateKind.Revoked:
            _LOGGER.error("The feeder no longer knows this Home Assistant: pair it again")
            self.revoked = True
            self._ready.set()
            self.connected = False
            if self.on_revoked is not None:
                self.on_revoked()
        elif kind == bridge.UpdateKind.Disconnected:
            if self.connected:
                _LOGGER.warning("Feeder disconnected: %s", update.reason)
            self.connected = False
        self._changed()

    @callback
    def _settled(self) -> None:
        """One lost feed is accounted for; the issue clears with the last."""
        self._uncertain = max(0, self._uncertain - 1)
        if self._uncertain == 0:
            ir.async_delete_issue(self.hass, DOMAIN, self._issue_id(UNCERTAIN_ISSUE))

    def _issue_id(self, issue: str) -> str:
        return f"{issue}_{self.identity.hex()}"

    @callback
    def _issue(self, issue: str, severity: ir.IssueSeverity, placeholders: dict[str, str]) -> None:
        ir.async_create_issue(
            self.hass,
            DOMAIN,
            self._issue_id(issue),
            is_fixable=False,
            is_persistent=False,
            severity=severity,
            translation_key=issue,
            translation_placeholders={"feeder": self.identity.hex()[:8], **placeholders},
        )

    @callback
    def _changed(self) -> None:
        for listener in list(self._listeners):
            listener()
