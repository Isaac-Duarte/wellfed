"""A stand-in for the bridge's `Feeder`, for paths the simulator cannot
reach from Python: a lost answer and its settlement, a key revoked before
setup, and a feeder that never answers.

The bridge's classes have no constructors in Python, so the fake hands out
plain objects with the same attributes, and the bridge's own enums.
"""

from __future__ import annotations

from dataclasses import dataclass, field
import queue
from types import SimpleNamespace
from typing import Any

import wellfed_homeassistant as bridge

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
    }
    values.update(changes)
    return SimpleNamespace(**values)


def update(kind: bridge.UpdateKind, **fields: Any) -> SimpleNamespace:
    values: dict[str, Any] = {
        "kind": kind,
        "build": None,
        "role": None,
        "capabilities": None,
        "state": None,
        "event": None,
        "replayed": False,
        "feed": None,
        "portions": None,
        "gap_from": None,
        "gap_to": None,
        "reason": None,
    }
    values.update(fields)
    return SimpleNamespace(**values)


def connected(role: bridge.Role = bridge.Role.Member) -> SimpleNamespace:
    capabilities = SimpleNamespace(
        feed=True, plans=True, desiccant=True, calibration=True, time=True, update=True
    )
    return update(
        bridge.UpdateKind.Connected, build="ab" * 32, role=role, capabilities=capabilities
    )


def written(result: bridge.WriteResult, **fields: Any) -> SimpleNamespace:
    values: dict[str, Any] = {"result": result, "feed": None, "refusal": None, "not_sent": None}
    values.update(fields)
    return SimpleNamespace(**values)


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

    def push(self, *updates: SimpleNamespace) -> None:
        for item in updates:
            self.updates.put(item)

    def next_update(self, timeout: float) -> SimpleNamespace | None:
        try:
            return self.updates.get(timeout=timeout)
        except queue.Empty:
            return None

    def _done(self, name: str, *args: Any) -> SimpleNamespace:
        self.calls.append((name, args))
        return written(bridge.WriteResult.Done)

    def feed(self, portions: int) -> SimpleNamespace:
        self.calls.append(("feed", (portions,)))
        return self.feed_result

    def cancel_feed(self, number: int) -> SimpleNamespace:
        return self._done("cancel_feed", number)

    def set_lock(self, locked: bool) -> SimpleNamespace:
        return self._done("set_lock", locked)
