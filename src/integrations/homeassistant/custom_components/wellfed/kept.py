"""What an entry keeps across restarts and reloads, in a store of its own.

- **Pending feeds:** each feed whose answer was lost, until an update
  settles it or the bridge stops waiting on it. The next session is handed
  them, so a feed lost before a restart is still settled rather than
  forgotten.
- **Answered feeds:** the feed numbers this Home Assistant has the end of,
  as the bridge last listed them: each feed it was answered for, and each
  lost feed a `Fed` event settled, as many as the bridge needs kept.
  Directly, a `Fed` event of one of them settles nothing, so the next
  session must know them too: a feed answered just before a lost one went
  out may end after it, and the replay of a `Fed` that settled one lost
  feed must not settle another.
- **A leftover key:** the client key an entry held directly before it moved
  to the server route, while the feeder may still list it.

One instance per entry, so a reload and a config flow write the same one.
The store is kept by the beta upgrade rule:

- It grows within its major as a minor (Home Assistant's `minor_version`)
  whose additions this build keeps unread, so an older build reads it, and
  saves it at the newer of the two minors, so a newer build never takes its
  own data for an older minor's.
- A newer integration's major (after a downgrade) is read as nothing and
  never written: the store is made read-only, and the entry starts empty.
- A file that does not read as this shape is copied aside under a numbered
  name, never deleted, and the entry starts empty. When the copy fails, the
  store is made read-only rather than saved over the file.
"""

from __future__ import annotations

import logging
import os
from pathlib import Path
from typing import Any, TypedDict, cast

from homeassistant.core import HomeAssistant
from homeassistant.exceptions import UnsupportedStorageVersionError
from homeassistant.helpers.storage import Store
from homeassistant.util.hass_dict import HassKey

import wellfed_homeassistant as bridge

from .const import DOMAIN

_LOGGER = logging.getLogger(__name__)

STORAGE_VERSION = 1
STORAGE_MINOR_VERSION = 1

# Copies set aside beside the store: never more, so a store that keeps
# failing cannot fill the disk.
ASIDE_MOST = 1000

STORES: HassKey[dict[str, Kept]] = HassKey(f"{DOMAIN}_kept")


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


def set_aside(path: str) -> str:
    """Copy the file at `path` to the first free `<path>.bad-<n>`, durably,
    and leave it in place for the next save to replace. Never replaces an
    earlier copy. The copy's path."""
    data = Path(path).read_bytes()
    for number in range(1, ASIDE_MOST + 1):
        aside = f"{path}.bad-{number}"
        try:
            with open(aside, "xb") as file:
                file.write(data)
                file.flush()
                os.fsync(file.fileno())
        except FileExistsError:
            continue
        return aside
    raise OSError(f"{ASIDE_MOST} copies of {path} are already set aside")


class _Store(Store[KeptData]):
    """The entry's store, reading any minor of its major as it is."""

    async def _async_migrate_func(
        self, old_major_version: int, old_minor_version: int, old_data: dict[str, Any]
    ) -> KeptData:
        if old_major_version != self.version:
            raise NotImplementedError
        # A minor that adds a field fills it in here for data an older minor
        # saved (`old_minor_version < STORAGE_MINOR_VERSION`), and a new
        # major converts the old one's data here too: Home Assistant runs no
        # other step. No minor has added one yet.
        # Home Assistant saves what this returns at the store's minor: keep a
        # newer one's, whose additions the data still carries.
        self.minor_version = max(self.minor_version, old_minor_version)
        return cast(KeptData, old_data)


class Kept:
    """One entry's store; changes are written at once, and again at shutdown."""

    def __init__(self, hass: HomeAssistant, entry_id: str) -> None:
        self._hass = hass
        self._store: Store[KeptData] = _Store(
            hass, STORAGE_VERSION, f"{DOMAIN}.{entry_id}", minor_version=STORAGE_MINOR_VERSION
        )
        self._data: KeptData = _empty()
        self._loaded = False

    async def load(self) -> None:
        """Read the store once; later calls keep what is in memory."""
        if self._loaded:
            return
        try:
            data: Any = await self._store.async_load()
        except UnsupportedStorageVersionError as error:
            self._store.make_read_only()
            _LOGGER.warning(
                "Wellfed's store for this entry is from a newer version of the integration "
                "and is left as it is (%s); until the integration is updated, the entry keeps "
                "no lost feeds across restarts",
                error,
            )
            data = None
        if data is None:
            self._data = _empty()
        elif _valid(data):
            self._data = data
        else:
            try:
                aside = await self._hass.async_add_executor_job(set_aside, self._store.path)
            except OSError as error:
                self._store.make_read_only()
                _LOGGER.warning(
                    "Wellfed's store for this entry is not in the shape it writes and could "
                    "not be set aside (%s); it is left as it is, nothing is saved over it, "
                    "and the entry starts with no lost feeds on record",
                    error,
                )
            else:
                _LOGGER.warning(
                    "Wellfed's store for this entry is not in the shape it writes; kept as "
                    "%s, and the entry starts with no lost feeds on record",
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

    def set_answered(self, feeds: list[int]) -> None:
        """The feed numbers to keep, as the bridge listed them."""
        self._data["answered"] = list(feeds)
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
