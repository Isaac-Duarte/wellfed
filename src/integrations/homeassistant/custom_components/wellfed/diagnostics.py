"""Diagnostics: what the session knows of the feeder, with the client
key's secret redacted."""

from __future__ import annotations

from typing import Any

from homeassistant.components.diagnostics import async_redact_data
from homeassistant.core import HomeAssistant

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .const import CONF_SECRET

REDACT = {CONF_SECRET}


def _status(status: bridge.Status | None) -> dict[str, Any] | None:
    if status is None:
        return None
    return {
        "build": status.build,
        "bank": status.bank,
        "uptime_ms": status.uptime_ms,
        "boot_count": status.boot_count,
        "rssi": status.rssi,
        "clock_ms": status.clock_ms,
        "locked": status.locked,
        "food": status.food.key,
        "readiness": status.readiness.key if status.readiness is not None else None,
        "feed_fault": (
            {
                "feed": status.feed_fault.feed,
                "outcome": status.feed_fault.outcome.key,
                "reason": status.feed_fault.reason.key,
            }
            if status.feed_fault is not None
            else None
        ),
        "adapter": status.adapter,
        "battery_percent": status.battery_percent,
        "below_floor": status.below_floor,
        "desiccant_days": status.desiccant_days,
        "last_event": status.last_event,
        "sessions": status.sessions,
    }


def _capabilities(capabilities: bridge.Capabilities | None) -> dict[str, bool] | None:
    if capabilities is None:
        return None
    return {
        "feed": capabilities.feed,
        "plans": capabilities.plans,
        "desiccant": capabilities.desiccant,
        "calibration": capabilities.calibration,
        "time": capabilities.time,
        "update": capabilities.update,
    }


async def async_get_config_entry_diagnostics(
    hass: HomeAssistant, entry: WellfedConfigEntry
) -> dict[str, Any]:
    session = entry.runtime_data
    return {
        "entry": async_redact_data(dict(entry.data), REDACT),
        "options": dict(entry.options),
        "connected": session.connected,
        "build": session.build,
        "role": session.role.key if session.role is not None else None,
        "capabilities": _capabilities(session.capabilities),
        "status": _status(session.status),
        "events": list(session.recent),
    }
