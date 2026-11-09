"""What an entry keeps across restarts and reloads, in a store of its own.

- **Pending feeds:** each feed whose answer was lost, until an update
  settles it. The next session is handed them, so a feed lost just before a
  restart is still settled (or found unknown) rather than forgotten, and
  the repair that names them stays in step.
- **Answered feeds:** the feed numbers this Home Assistant was answered
  for since the oldest pending feed. Directly, a `Fed` event of one of
  them settles nothing, so the next session must know them too.
- **A leftover key:** the client key an entry held directly before it moved
  to the server route, while the feeder may still list it.

One instance per entry, so a reload and a config flow write the same one.
"""

from __future__ import annotations

from typing import TypedDict

from homeassistant.core import HomeAssistant
from homeassistant.helpers.storage import Store
from homeassistant.util.hass_dict import HassKey

import wellfed_homeassistant as bridge

from .const import DOMAIN

STORAGE_VERSION = 1

STORES: HassKey[dict[str, Kept]] = HassKey(f"{DOMAIN}_kept")

# Answered feed numbers kept while a feed is pending: far more than a
# settle window holds.
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


class Kept:
    """One entry's store; changes are written at once, and again at shutdown."""

    def __init__(self, hass: HomeAssistant, entry_id: str) -> None:
        self._store: Store[KeptData] = Store(hass, STORAGE_VERSION, f"{DOMAIN}.{entry_id}")
        self._data: KeptData = _empty()
        self._loaded = False

    async def load(self) -> None:
        """Read the store once; later calls keep what is in memory."""
        if self._loaded:
            return
        self._data = await self._store.async_load() or _empty()
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
        """`pending` is accounted for; the answered feeds go with the last."""
        self._data["pending"] = [p for p in self._data["pending"] if p["id"] != pending.id]
        if not self._data["pending"]:
            self._data["answered"] = []
        self._save()

    def answered_feed(self, feed: int) -> None:
        """A feed this Home Assistant was answered for, kept while any is pending."""
        if not self._data["pending"]:
            return
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
