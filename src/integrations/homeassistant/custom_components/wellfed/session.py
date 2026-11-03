"""One feeder's session, as Home Assistant sees it.

The Rust bridge owns the connection, the keepalive and the event cursor on
a thread of its own. This class pumps the bridge's updates into the event
loop and hands writes to the executor. It holds no feeding logic: a write
goes out once through the bridge, and a write whose answer never came is
reported as uncertain and never sent again.
"""

from __future__ import annotations

from collections.abc import Callable
import logging
import threading

from homeassistant.core import HomeAssistant, callback
from homeassistant.exceptions import HomeAssistantError
from homeassistant.util import dt as dt_util

import wellfed_homeassistant as bridge

from .const import EVENT_FEEDER

_LOGGER = logging.getLogger(__name__)

# How long the pump waits on the bridge before it looks for a stop.
PUMP_WAIT = 1.0


class WriteNotDone(HomeAssistantError):
    """A feed or lock change the feeder did not carry out, or may have."""


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
    """The bridge's session, its latest status and its listeners."""

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
        self.build: str | None = None
        self.role: bridge.Role | None = None
        self.status: bridge.Status | None = None
        self.last_fed: bridge.Feed | None = None

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

    @callback
    def add_listener(self, listener: Callable[[], None]) -> Callable[[], None]:
        """Call `listener` on every change; returns its removal."""
        self._listeners.append(listener)
        return lambda: self._listeners.remove(listener)

    async def feed(self, portions: int) -> int:
        """Start a feed; the feeder's number for it."""
        written = await self._write(lambda feeder: feeder.feed(portions))
        if written.feed is None:
            raise WriteNotDone("The feeder answered a feed without its number")
        return written.feed

    async def set_lock(self, locked: bool) -> None:
        """Lock or unlock the panel."""
        await self._write(lambda feeder: feeder.set_lock(locked))

    async def _write(self, call: Callable[[bridge.Feeder], bridge.Written]) -> bridge.Written:
        feeder = self._feeder
        if feeder is None:
            raise WriteNotDone("Not sent: the session has stopped")
        written = await self.hass.async_add_executor_job(call, feeder)
        result = written.result
        if result in (bridge.WriteResult.Feeding, bridge.WriteResult.Done):
            return written
        if result == bridge.WriteResult.Uncertain:
            # It may have run. The feeder's event settles it; sending it
            # again could feed twice.
            raise WriteNotDone(
                "The feeder did not answer, so it may have run. It was not "
                "sent again; the feeding history will show whether it did."
            )
        if result == bridge.WriteResult.NotSent:
            raise WriteNotDone(f"Not sent: {written.reason}")
        refusal = written.refusal.key if written.refusal is not None else "unknown"
        raise WriteNotDone(f"The feeder refused: {refusal}")

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
        elif kind == bridge.UpdateKind.Status:
            self.status = update.status
        elif kind == bridge.UpdateKind.Event and update.event is not None:
            event = update.event
            if event.fed is not None:
                self.last_fed = event.fed
            self.hass.bus.async_fire(EVENT_FEEDER, event_data(self.identity, event))
        elif kind == bridge.UpdateKind.Gap:
            _LOGGER.info(
                "Some feeder events were not received: %s to %s", update.gap_from, update.gap_to
            )
        elif kind == bridge.UpdateKind.Disconnected:
            if self.connected:
                _LOGGER.warning("Feeder disconnected: %s", update.reason)
            self.connected = False
        for listener in list(self._listeners):
            listener()
