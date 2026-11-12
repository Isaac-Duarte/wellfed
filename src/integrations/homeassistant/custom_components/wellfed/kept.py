"""What an entry keeps across restarts and reloads, in a store of its own.

- **Pending feeds:** each direct feed from just before it goes out, and
  each feed whose answer was lost, until its answer clears it, an update
  settles it, or the bridge stops waiting on it; with whether it was
  reported still unknown, when the owner released it, and through the
  server the token it went under. The next session is handed them, on
  either route, so a feed lost before a restart, or out when Home Assistant
  stopped, is still settled rather than forgotten, one already reported is
  not reported again, and one not released still holds new feeds back,
  whichever route the entry is on now.
- **Answered feeds:** the feed numbers this Home Assistant has the end of,
  as the bridge last listed them: each feed it was answered for, and each
  lost feed a `Fed` event settled, as many as the bridge needs kept.
  Directly, a `Fed` event of one of them settles nothing, so the next
  session must know them too: a feed answered just before a lost one went
  out may end after it, and the replay of a `Fed` that settled one lost
  feed must not settle another.
- **Leftover keys:** each client key an entry held directly before it moved
  to the server route, while the feeder may still list it.
- **Unread:** the store before this one did not read, so a lost feed it
  kept is lost with it; every feed waits until the owner says the bowls
  are checked.

One instance per entry, so a reload and a config flow write the same one.
The store is kept by the beta upgrade rule:

- It grows within its major as a minor (Home Assistant's `minor_version`)
  whose additions this build keeps unread, so an older build reads it, and
  saves it at the newer of the two minors, so a newer build never takes its
  own data for an older minor's.
- A newer integration's major (after a downgrade) is read as nothing and
  never written: the store is made read-only, and the entry starts empty
  and unread.
- A file that does not read as this shape is copied aside under a numbered
  name, never deleted, and the entry starts empty and unread. When the copy
  fails, the store is made read-only rather than saved over the file.
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
    reported: bool
    released_ms: int | None
    token_id: str | None


class KeptData(TypedDict):
    pending: list[PendingData]
    answered: list[int]
    leftover_keys: list[str]
    unread: bool


def _empty(unread: bool = False) -> KeptData:
    return {"pending": [], "answered": [], "leftover_keys": [], "unread": unread}


def _int(value: Any, low: int, high: int) -> bool:
    """Whether `value` is an integer from `low` to `high`, as the bridge takes it."""
    return isinstance(value, int) and not isinstance(value, bool) and low <= value <= high


def _key(value: Any) -> bool:
    """Whether `value` is a client key in hex, 32 bytes."""
    if not isinstance(value, str) or len(value) != 64:
        return False
    try:
        bytes.fromhex(value)
    except ValueError:
        return False
    return True


def _pending(item: Any) -> bool:
    return (
        isinstance(item, dict)
        and isinstance(item.get("id"), str)
        and _int(item.get("portions"), bridge.PORTIONS_MIN, bridge.PORTIONS_MAX)
        and _int(item.get("sent_ms"), -(2**63), 2**63 - 1)
        and _optional(item, "after", 0, 2**64 - 1)
        and isinstance(item.get("reported"), bool)
        and _optional(item, "released_ms", -(2**63), 2**63 - 1)
        and "token_id" in item
        and (item["token_id"] is None or isinstance(item["token_id"], str))
    )


def _optional(item: dict[str, Any], name: str, low: int, high: int) -> bool:
    """Whether `item` has `name`, null or an integer from `low` to `high`: a
    missing one would fail setup when the bridge is handed it."""
    if name not in item:
        return False
    value = item[name]
    return value is None or _int(value, low, high)


def _valid(data: Any) -> bool:
    """Whether `data` is the store's shape, every field typed and within what
    the bridge takes: a value past it would fail setup on every try rather
    than set the file aside."""
    return (
        isinstance(data, dict)
        and isinstance(data.get("pending"), list)
        and all(_pending(item) for item in data["pending"])
        and isinstance(data.get("answered"), list)
        and all(_int(feed, 0, 2**32 - 1) for feed in data["answered"])
        and isinstance(data.get("leftover_keys"), list)
        and all(_key(key) for key in data["leftover_keys"])
        and isinstance(data.get("unread"), bool)
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
                "no lost feeds across restarts, and feeds wait at each start until the "
                "bowls are checked",
                error,
            )
            self._data = _empty(unread=True)
            self._loaded = True
            return
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
                    "and feeds wait at each start until the bowls are checked",
                    error,
                )
                self._data = _empty(unread=True)
            else:
                _LOGGER.warning(
                    "Wellfed's store for this entry is not in the shape it writes; kept as "
                    "%s, and feeds wait until the bowls are checked",
                    aside,
                )
                # Saved at once, so a restart finds the hold, not the file.
                self._data = _empty(unread=True)
                self._save()
        self._loaded = True

    @property
    def pending(self) -> list[bridge.Pending]:
        return [
            bridge.Pending(
                item["id"],
                item["portions"],
                item["sent_ms"],
                item["after"],
                item["reported"],
                item["released_ms"],
                item["token_id"],
            )
            for item in self._data["pending"]
        ]

    @property
    def answered(self) -> list[int]:
        return list(self._data["answered"])

    @property
    def leftover_keys(self) -> list[bytes]:
        return [bytes.fromhex(key) for key in self._data["leftover_keys"]]

    @property
    def unread(self) -> bool:
        """The store before this one did not read, and the bowls are not yet
        checked: every feed waits."""
        return self._data["unread"]

    def add_pending(self, pending: bridge.Pending) -> None:
        if any(item["id"] == pending.id for item in self._data["pending"]):
            return
        self._data["pending"].append(
            {
                "id": pending.id,
                "portions": pending.portions,
                "sent_ms": pending.sent_ms,
                "after": pending.after,
                "reported": pending.reported,
                "released_ms": pending.released_ms,
                "token_id": pending.token_id,
            }
        )
        self._save()

    def mark_reported(self, pending: bridge.Pending) -> None:
        """`pending` was reported still unknown."""
        for item in self._data["pending"]:
            if item["id"] == pending.id:
                item["reported"] = True
        self._save()

    def mark_released(self, pending: bridge.Pending) -> None:
        """The owner released `pending`, as the bridge confirmed."""
        for item in self._data["pending"]:
            if item["id"] == pending.id:
                item["released_ms"] = pending.released_ms
        self._save()

    def settle(self, pending: bridge.Pending) -> None:
        """`pending` is accounted for."""
        self._data["pending"] = [p for p in self._data["pending"] if p["id"] != pending.id]
        self._save()

    def set_answered(self, feeds: list[int]) -> None:
        """The feed numbers to keep, as the bridge listed them."""
        self._data["answered"] = list(feeds)
        self._save()

    def add_leftover_key(self, key: bytes) -> None:
        """`key` may still be listed on the feeder: kept until it is not."""
        if key.hex() not in self._data["leftover_keys"]:
            self._data["leftover_keys"].append(key.hex())
            self._save()

    def remove_leftover_key(self, key: bytes) -> None:
        self._data["leftover_keys"] = [k for k in self._data["leftover_keys"] if k != key.hex()]
        self._save()

    def bowls_checked(self) -> None:
        """The owner checked the bowls, as the bridge confirmed: feeds go again."""
        self._data["unread"] = False
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
