"""What an entry keeps across restarts and reloads, in a store of its own.

- **Pending feeds:** each feed whose answer was lost, until an update
  settles it. The next session is handed them, so a feed lost just before a
  restart is still settled (or found unknown) rather than forgotten, and
  the repair that names them stays in step.
- **Answered feeds:** the last feed numbers this Home Assistant was
  answered for, pending or not. Directly, a `Fed` event of one of them
  settles nothing, so the next session must know them too: a feed answered
  just before a lost one went out may end after it.
- **A leftover key:** the client key an entry held directly before it moved
  to the server route, while the feeder may still list it.

One instance per entry, so a reload and a config flow write the same one.
A file that does not read as this shape is set aside, never deleted, and
the entry starts empty.
"""

from __future__ import annotations

import logging
import os
import time
from typing import Any, TypedDict

from homeassistant.core import HomeAssistant
from homeassistant.helpers.storage import Store
from homeassistant.util.hass_dict import HassKey

import wellfed_homeassistant as bridge

from .const import DOMAIN

_LOGGER = logging.getLogger(__name__)

STORAGE_VERSION = 1

STORES: HassKey[dict[str, Kept]] = HassKey(f"{DOMAIN}_kept")

# Answered feed numbers kept: far more than a settle window holds.
ANSWERED_KEPT = 64


class PendingData(TypedDict):
    id: str
    portions: int
    sent_ms: int
    after: int | None


class KeptData(TypedDict):
    pending: list[PendingData]
    answered: list[int]
    leftover_key: str | None


def _empty() -> KeptData:
    return {"pending": [], "answered": [], "leftover_key": None}


def _int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _pending(item: Any) -> bool:
    return (
        isinstance(item, dict)
        and isinstance(item.get("id"), str)
        and _int(item.get("portions"))
        and _int(item.get("sent_ms"))
        and (item.get("after") is None or _int(item.get("after")))
    )


def _valid(data: Any) -> bool:
    """Whether `data` is the store's shape, every field typed."""
    return (
        isinstance(data, dict)
        and isinstance(data.get("pending"), list)
        and all(_pending(item) for item in data["pending"])
        and isinstance(data.get("answered"), list)
        and all(_int(feed) for feed in data["answered"])
        and (data.get("leftover_key") is None or isinstance(data["leftover_key"], str))
    )


class Kept:
    """One entry's store; changes are written at once, and again at shutdown."""

    def __init__(self, hass: HomeAssistant, entry_id: str) -> None:
        self._hass = hass
        self._store: Store[KeptData] = Store(hass, STORAGE_VERSION, f"{DOMAIN}.{entry_id}")
        self._data: KeptData = _empty()
        self._loaded = False

    async def load(self) -> None:
        """Read the store once; later calls keep what is in memory."""
        if self._loaded:
            return
        data: Any = await self._store.async_load()
        if data is None:
            self._data = _empty()
        elif _valid(data):
            self._data = data
        else:
            aside = f"{self._store.path}.bad-{int(time.time())}"
            try:
                await self._hass.async_add_executor_job(os.replace, self._store.path, aside)
            except OSError as error:
                _LOGGER.warning("Could not set Wellfed's store aside as %s: %s", aside, error)
            _LOGGER.warning(
                "Wellfed's store for this entry is not in the shape it writes; kept as %s, "
                "and the entry starts with no lost feeds on record",
                aside,
            )
            self._data = _empty()
        self._loaded = True

    @property
    def pending(self) -> list[bridge.Pending]:
        return [
            bridge.Pending(item["id"], item["portions"], item["sent_ms"], item["after"])
            for item in self._data["pending"]
        ]

    @property
    def answered(self) -> list[int]:
        return list(self._data["answered"])

    @property
    def leftover_key(self) -> bytes | None:
        key = self._data["leftover_key"]
        return bytes.fromhex(key) if key is not None else None

    def add_pending(self, pending: bridge.Pending) -> None:
        if any(item["id"] == pending.id for item in self._data["pending"]):
            return
        self._data["pending"].append(
            {
                "id": pending.id,
                "portions": pending.portions,
                "sent_ms": pending.sent_ms,
                "after": pending.after,
            }
        )
        self._save()

    def settle(self, pending: bridge.Pending) -> None:
        """`pending` is accounted for."""
        self._data["pending"] = [p for p in self._data["pending"] if p["id"] != pending.id]
        self._save()

    def answered_feed(self, feed: int) -> None:
        """A feed this Home Assistant was answered for: the last ones are kept."""
        self._data["answered"] = [*self._data["answered"], feed][-ANSWERED_KEPT:]
        self._save()

    def set_leftover_key(self, key: bytes | None) -> None:
        self._data["leftover_key"] = key.hex() if key is not None else None
        self._save()

    def _save(self) -> None:
        self._store.async_delay_save(lambda: self._data, 0)

    async def remove(self) -> None:
        await self._store.async_remove()


def kept(hass: HomeAssistant, entry_id: str) -> Kept:
    """The entry's one store."""
    stores = hass.data.setdefault(STORES, {})
    if entry_id not in stores:
        stores[entry_id] = Kept(hass, entry_id)
    return stores[entry_id]


async def remove(hass: HomeAssistant, entry_id: str) -> None:
    """Remove the entry's store, in memory and on disk."""
    store = hass.data.setdefault(STORES, {}).pop(entry_id, None) or Kept(hass, entry_id)
    await store.remove()
