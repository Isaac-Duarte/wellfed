"""One feeder's session, as Home Assistant sees it.

The Rust bridge owns the connection, the keepalive and the event cursor on
a thread of its own. This class pumps the bridge's updates into the event
loop and hands writes to the executor. It holds no feeding logic: a write
goes out once through the bridge, and a write whose answer never came is
reported as uncertain and never sent again.
"""

from __future__ import annotations

from collections.abc import Callable
import json
import logging
import threading
from typing import Any

from homeassistant.core import HomeAssistant, callback
from homeassistant.exceptions import HomeAssistantError

import wellfed_homeassistant as bridge

from .const import EVENT_FEEDER

_LOGGER = logging.getLogger(__name__)

# How long the pump waits on the bridge before it looks for a stop.
PUMP_WAIT = 1.0


class WriteNotDone(HomeAssistantError):
    """A feed or lock change the feeder did not carry out, or may have."""


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
        self._feeder = bridge.Feeder(secret, identity, address)
        self._stop = threading.Event()
        self._pump: threading.Thread | None = None
        self._listeners: list[Callable[[], None]] = []

        self.connected = False
        self.build: str | None = None
        self.role: str | None = None
        self.status: dict[str, Any] | None = None
        self.last_fed: dict[str, Any] | None = None

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
        written = await self._write(self._feeder.feed, portions)
        return written["feed"]

    async def set_lock(self, locked: bool) -> None:
        """Lock or unlock the panel."""
        await self._write(self._feeder.set_lock, locked)

    async def _write(self, call: Callable[[Any], str], value: Any) -> dict[str, Any]:
        written = json.loads(await self.hass.async_add_executor_job(call, value))
        result = written["result"]
        if result in ("feeding", "done"):
            return written
        if result == "uncertain":
            # It may have run. The feeder's event settles it; sending it
            # again could feed twice.
            raise WriteNotDone(
                "The feeder did not answer, so it may have run. It was not "
                "sent again; the feeding history will show whether it did."
            )
        if result == "not_sent":
            raise WriteNotDone(f"Not sent: {written['reason']}")
        raise WriteNotDone(f"The feeder refused: {written['refusal']}")

    def _run(self) -> None:
        while not self._stop.is_set():
            feeder = self._feeder
            if feeder is None:
                return
            text = feeder.next_update(PUMP_WAIT)
            if text is not None:
                self.hass.loop.call_soon_threadsafe(self._apply, json.loads(text))

    @callback
    def _apply(self, update: dict[str, Any]) -> None:
        kind = update["kind"]
        if kind == "connected":
            self.connected = True
            self.build = update["build"]
            self.role = update["role"]
        elif kind == "status":
            self.status = update["status"]
        elif kind == "event":
            event = update["event"]
            if event["body"]["kind"] == "fed":
                self.last_fed = event
            self.hass.bus.async_fire(
                EVENT_FEEDER,
                {"identity": self.identity.hex(), **event},
            )
        elif kind == "gap":
            _LOGGER.info("Some feeder events were not received: %s", update["gap"])
        elif kind == "disconnected":
            if self.connected:
                _LOGGER.warning("Feeder disconnected: %s", update["reason"])
            self.connected = False
        for listener in list(self._listeners):
            listener()
