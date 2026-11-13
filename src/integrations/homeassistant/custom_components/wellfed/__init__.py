"""The Wellfed integration: a feeder reached directly over its device API, as a
member Home Assistant joined with an invite."""

from __future__ import annotations

import logging

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import ATTR_DEVICE_ID, Platform
from homeassistant.core import HomeAssistant, ServiceCall, ServiceResponse, SupportsResponse
from homeassistant.exceptions import (
    ConfigEntryAuthFailed,
    ConfigEntryError,
    ConfigEntryNotReady,
    ServiceValidationError,
)
from homeassistant.helpers import (
    config_validation as cv,
    device_registry as dr,
    issue_registry as ir,
)
from homeassistant.helpers.typing import ConfigType
import probatio

import wellfed_homeassistant as bridge

from .const import (
    ATTR_FEED,
    ATTR_PORTIONS,
    CONF_IDENTITY,
    CONF_SECRET,
    DOMAIN,
    ENTRY_VERSION,
    PORTIONS_MAX,
    PORTIONS_MIN,
    SERVICE_CANCEL_FEED,
    SERVICE_FEED,
)
from .kept import kept, remove as remove_kept
from .session import (
    STORE_READ_ONLY_ISSUE,
    STORE_UNREAD_ISSUE,
    UNCERTAIN_ISSUE,
    UNKNOWN_ISSUE,
    FeederSession,
    hold_dropped,
    open_feeder,
)

PLATFORMS = [
    Platform.BINARY_SENSOR,
    Platform.BUTTON,
    Platform.EVENT,
    Platform.NUMBER,
    Platform.SENSOR,
    Platform.SWITCH,
]

# How long setup waits for the feeder's first answer before it tries again
# later.
READY_WAIT = 15.0

_LOGGER = logging.getLogger(__name__)

type WellfedConfigEntry = ConfigEntry[FeederSession]

CONFIG_SCHEMA = cv.config_entry_only_config_schema(DOMAIN)

FEED_SCHEMA = probatio.Schema(
    {
        probatio.Required(ATTR_DEVICE_ID): cv.string,
        probatio.Optional(ATTR_PORTIONS, default=1): probatio.All(
            probatio.Coerce(int), probatio.Range(min=PORTIONS_MIN, max=PORTIONS_MAX)
        ),
    }
)
CANCEL_SCHEMA = probatio.Schema(
    {
        probatio.Required(ATTR_DEVICE_ID): cv.string,
        probatio.Optional(ATTR_FEED): probatio.All(probatio.Coerce(int), probatio.Range(min=0)),
    }
)


def _entry(hass: HomeAssistant, call: ServiceCall) -> WellfedConfigEntry:
    """The loaded entry of the feeder device an action names."""
    device = dr.async_get(hass).async_get(call.data[ATTR_DEVICE_ID])
    for entry in hass.config_entries.async_loaded_entries(DOMAIN):
        if device is not None and entry.entry_id in device.config_entries:
            return entry
    raise ServiceValidationError(translation_domain=DOMAIN, translation_key="not_a_feeder")


async def async_setup(hass: HomeAssistant, config: ConfigType) -> bool:
    """Register the actions, which name the feeder by its device."""

    async def feed(call: ServiceCall) -> ServiceResponse:
        session = _entry(hass, call).runtime_data
        return {"feed": await session.feed(call.data[ATTR_PORTIONS])}

    async def cancel_feed(call: ServiceCall) -> None:
        session = _entry(hass, call).runtime_data
        feed = call.data.get(ATTR_FEED, session.running_feed)
        if feed is None:
            raise ServiceValidationError(translation_domain=DOMAIN, translation_key="no_feed")
        await session.cancel_feed(feed)

    hass.services.async_register(
        DOMAIN,
        SERVICE_FEED,
        feed,
        schema=FEED_SCHEMA,
        supports_response=SupportsResponse.OPTIONAL,
    )
    hass.services.async_register(DOMAIN, SERVICE_CANCEL_FEED, cancel_feed, schema=CANCEL_SCHEMA)
    return True


async def async_migrate_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    """A version 1 entry, from early test builds, is not carried over: it fails
    to migrate, and the owner deletes it and pairs again. A newer minor of
    version 2 grew only by what this build reads past, so it is read as it is
    and left at its minor; a newer major is refused before this is asked."""
    if entry.version == ENTRY_VERSION:
        return True
    if entry.version == 1:
        _LOGGER.error(
            "This Wellfed entry is from an earlier test build: delete it and pair the feeder again"
        )
    return False


async def async_setup_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    # An entry with no client key was made for the owner's server, which
    # Wellfed no longer has: nothing here reaches the feeder for it, so the
    # owner deletes it and adds the feeder directly.
    if CONF_SECRET not in entry.data:
        raise ConfigEntryError(translation_domain=DOMAIN, translation_key="no_client_key")
    store = kept(hass, entry.entry_id)
    await store.load()
    try:
        feeder = await hass.async_add_executor_job(
            open_feeder, entry.data, store.pending, store.answered, store.unread
        )
    except bridge.WellfedError as error:
        raise ConfigEntryError(translation_domain=DOMAIN, translation_key="bad_entry") from error
    session = FeederSession(hass, feeder, bytes.fromhex(entry.data[CONF_IDENTITY]), store)
    session.on_revoked = lambda: entry.async_start_reauth(hass)
    session.start()

    # The first state names the role and what the feeder serves, which
    # decide the entities.
    await session.ready(READY_WAIT)
    if session.revoked or session.state is None:
        await session.stop()
        if session.revoked:
            raise ConfigEntryAuthFailed(translation_domain=DOMAIN, translation_key="revoked")
        raise ConfigEntryNotReady(translation_domain=DOMAIN, translation_key="not_ready")

    entry.runtime_data = session
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    unloaded = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if unloaded:
        await entry.runtime_data.stop()
    return unloaded


async def async_remove_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> None:
    """The client key stays on the feeder until the owner removes it in the app, since
    a member may not remove its own. A hold on feeds goes with the entry, and a repair
    says so: a new entry for the feeder starts without it."""
    if identity := entry.data.get(CONF_IDENTITY):
        feeder = bytes.fromhex(identity)
        store = kept(hass, entry.entry_id)
        await store.load()
        for issue in (UNCERTAIN_ISSUE, UNKNOWN_ISSUE, STORE_UNREAD_ISSUE, STORE_READ_ONLY_ISSUE):
            ir.async_delete_issue(hass, DOMAIN, f"{issue}_{identity}")
        if store.unread or any(p.released_ms is None for p in store.pending):
            hold_dropped(hass, feeder)
    await remove_kept(hass, entry.entry_id)


async def async_remove_config_entry_device(
    hass: HomeAssistant, entry: WellfedConfigEntry, device: dr.DeviceEntry
) -> bool:
    """A device of this entry other than its feeder is stale, and may go."""
    return (DOMAIN, entry.data[CONF_IDENTITY]) not in device.identifiers
