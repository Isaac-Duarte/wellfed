"""A stand-in for the bridge's `Feeder`, for paths the simulator cannot
reach from Python: a lost answer and its settlement, a key revoked before
setup, and a feeder that never answers.

The bridge's classes have no constructors in Python, so the fake hands out
plain objects with the same attributes, and the bridge's own enums.
"""

from __future__ import annotations

from dataclasses import dataclass, field
import queue
import threading
import time
from types import SimpleNamespace
from typing import Any

import wellfed_homeassistant as bridge

# How long a pump held for a stop waits for it.
STOP_WAIT = 30.0

IDENTITY = bytes(range(32))
SECRET = bytes([7] * 32)


def status(**changes: Any) -> SimpleNamespace:
    values: dict[str, Any] = {
        "build": "ab" * 32,
        "bank": 1,
        "uptime_ms": 1000,
        "boot_count": 3,
        "rssi": -50,
        "clock_ms": 1_793_793_600_000,
        "locked": False,
        "food": bridge.FoodLevel.Present,
        "readiness": bridge.Readiness.Ready,
        "feed_fault": None,
        "adapter": True,
        "battery_percent": 90,
        "below_floor": False,
        "desiccant_days": 20,
        "desiccant_due": False,
        "last_event": 10,
        "sessions": 1,
    }
    values.update(changes)
    return SimpleNamespace(**values)


def state(**changes: Any) -> SimpleNamespace:
    values: dict[str, Any] = {
        "status": status(),
        "settings": SimpleNamespace(
            sound=True, lights=True, volume=50, desiccant_days=30, meal_reminder_minutes=None
        ),
        "plans": [],
        "next_meal": None,
        "last_fed": None,
        "portions_today": None,
        "portion_mg": 9000,
        "time_server_disagrees": False,
    }
    values.update(changes)
    return SimpleNamespace(**values)


def plan(slot: int) -> SimpleNamespace:
    """A daily plan in `slot`, on."""
    return SimpleNamespace(slot=slot, minute=480, portions=1, days=0x7F, enabled=True)


def update(kind: bridge.UpdateKind, **fields: Any) -> SimpleNamespace:
    values: dict[str, Any] = {
        "kind": kind,
        "build": None,
        "role": None,
        "capabilities": None,
        "skew": None,
        "state": None,
        "event": None,
        "replayed": False,
        "feed": None,
        "portions": None,
        "pending": None,
        "gap_from": None,
        "gap_to": None,
        "reason": None,
        "alerts": None,
        "answered": [],
    }
    values.update(fields)
    return SimpleNamespace(**values)


def alert(key: str, kind: bridge.AlertKind, level: bridge.AlertLevel) -> SimpleNamespace:
    """One alert, as the bridge hands it out."""
    return SimpleNamespace(key=key, kind=kind, level=level, at_ms=1_793_793_600_000)


def alerts(
    open: list[SimpleNamespace],
    opened: list[SimpleNamespace] | None = None,
    cleared: list[SimpleNamespace] | None = None,
) -> SimpleNamespace:
    """An alerts update: what is open, and what opened and cleared."""
    return update(
        bridge.UpdateKind.Alerts,
        alerts=SimpleNamespace(open=open, opened=opened or [], cleared=cleared or [], unread=0),
    )


def connected(
    role: bridge.Role = bridge.Role.Member, skew: bridge.Skew = bridge.Skew.Same
) -> SimpleNamespace:
    capabilities = SimpleNamespace(
        feed=True, plans=True, desiccant=True, calibration=True, time=True, update=True
    )
    return update(
        bridge.UpdateKind.Connected,
        build="ab" * 32,
        role=role,
        capabilities=capabilities,
        skew=skew,
    )


def written(result: bridge.WriteResult, **fields: Any) -> SimpleNamespace:
    values: dict[str, Any] = {
        "result": result,
        "feed": None,
        "refusal": None,
        "not_sent": None,
        "pending": None,
    }
    values.update(fields)
    return SimpleNamespace(**values)


def pending(id: str = "a-lost-feed", portions: int = 1) -> bridge.Pending:
    """A feed whose answer was lost, as the bridge hands it out."""
    return bridge.Pending(id, portions, 1_793_793_600_000, None, False, None)


def uncertain(lost: bridge.Pending) -> SimpleNamespace:
    """A feed write whose answer was lost, kept as `lost`."""
    return written(bridge.WriteResult.Uncertain, pending=lost)


def feed(number: int, outcome: bridge.FeedOutcome = bridge.FeedOutcome.Done) -> SimpleNamespace:
    return SimpleNamespace(
        feed=number,
        at_ms=1_793_793_600_000,
        requested=1,
        completed=1,
        at_most=1,
        outcome=outcome,
        reason=bridge.FeedReason.Done,
        hopper=bridge.FoodLevel.Present,
        by=bridge.By.Client,
        client="01020304",
        plan=None,
    )


@dataclass
class FakeFeeder:
    """Scripted: what it pushes, and how a feed ends."""

    updates: queue.Queue[SimpleNamespace] = field(default_factory=queue.Queue)
    feed_result: SimpleNamespace = field(
        default_factory=lambda: written(bridge.WriteResult.Feeding, feed=1)
    )
    calls: list[tuple[str, tuple[Any, ...]]] = field(default_factory=list)
    # When set, a feed waits for it: a write still out when the test acts.
    gate: threading.Event | None = None
    # The feed numbers it was answered for, as the bridge lists them.
    answered: list[int] = field(default_factory=list)

    # Updates held for a stop (`at_stop`): the pump gets the first only once
    # `stopping` is set, and the stop's drain the rest.
    held: list[SimpleNamespace] = field(default_factory=list)
    stopping: threading.Event | None = None

    # The store's word on each feed's record, by its id, as the session gave
    # it: whether it saved it.
    words: dict[str, bool] = field(default_factory=dict)
    said: threading.Condition = field(default_factory=threading.Condition)

    def push(self, *updates: SimpleNamespace) -> None:
        for item in updates:
            self.updates.put(item)

    def at_stop(self, stopping: threading.Event, *updates: SimpleNamespace) -> None:
        """Hold `updates` until the session stops: the pump hands the event
        loop the first of them only once `stopping` is set, so it is still
        to run when the stop begins, and leaves the rest to the drain."""
        self.held = list(updates)
        self.stopping = stopping

    def next_update(self, timeout: float) -> SimpleNamespace | None:
        if timeout == 0:
            # The stop's drain, once the pump has ended.
            if self.held:
                return self.held.pop(0)
            try:
                return self.updates.get_nowait()
            except queue.Empty:
                return None
        deadline = time.monotonic() + timeout
        while True:
            if (stopping := self.stopping) is not None:
                # The pump returns nothing until the stop, so it cannot
                # miss it between two waits.
                if not stopping.wait(STOP_WAIT):
                    raise AssertionError(f"the session did not stop within {STOP_WAIT} s")
                self.stopping = None
                return self.held.pop(0)
            try:
                return self.updates.get(timeout=min(0.01, max(0.0, deadline - time.monotonic())))
            except queue.Empty:
                if time.monotonic() >= deadline:
                    return None

    def _done(self, name: str, *args: Any) -> SimpleNamespace:
        self.calls.append((name, args))
        return written(bridge.WriteResult.Done)

    def feed(self, portions: int) -> SimpleNamespace:
        self.calls.append(("feed", (portions,)))
        if self.gate is not None:
            self.gate.wait()
        # As the bridge does: the feed's record first, and the feed only once
        # the store says it landed; dropped again if it did not.
        out = self.feed_result.pending or bridge.Pending(
            f"sending-{len(self.calls)}", portions, 0, None, False, None
        )
        self.push(update(bridge.UpdateKind.Sending, pending=out))
        with self.said:
            self.said.wait_for(lambda: out.id in self.words, timeout=STOP_WAIT)
            kept = self.words.get(out.id, False)
        if not kept:
            self.push(update(bridge.UpdateKind.Cleared, pending=out))
            return written(bridge.WriteResult.NotSent, not_sent=bridge.NotSent.NotSaved)
        # A lost feed, or an answered feed's number, is announced before its
        # answer; an answered feed's record goes.
        if self.feed_result.pending is not None:
            self.push(update(bridge.UpdateKind.Pending, pending=self.feed_result.pending))
        else:
            self.push(update(bridge.UpdateKind.Cleared, pending=out))
        if self.feed_result.feed is not None:
            self.answered.append(self.feed_result.feed)
            self.push(update(bridge.UpdateKind.Answered, answered=list(self.answered)))
        return self.feed_result

    def cancel_feed(self, number: int) -> SimpleNamespace:
        return self._done("cancel_feed", number)

    def set_lock(self, locked: bool) -> SimpleNamespace:
        return self._done("set_lock", locked)

    def release(self, pending_id: str) -> None:
        """The owner released a lost feed: the test confirms it, as the
        bridge does, with a `Released` update."""
        self.calls.append(("release", (pending_id,)))

    def saved(self, pending_id: str) -> None:
        with self.said:
            self.words[pending_id] = True
            self.said.notify_all()

    def unsaved(self, pending_id: str) -> None:
        with self.said:
            self.words[pending_id] = False
            self.said.notify_all()

    def bowls_checked(self) -> None:
        """The owner checked the bowls: the test confirms it, as the bridge
        does, with a `BowlsChecked` update."""
        self.calls.append(("bowls_checked", ()))
