"""The Wellfed integration: a feeder reached directly, with no server, or
through the owner's Wellfed server; the entities are the same either way."""

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
    CONF_ADDRESS,
    CONF_FEEDER_ID,
    CONF_IDENTITY,
    CONF_ROUTE,
    CONF_SECRET,
    CONF_TOKEN,
    CONF_TOKEN_ID,
    CONF_URL,
    DOMAIN,
    ENTRY_VERSION,
    LEFTOVER_KEY_LEFT,
    PORTIONS_MAX,
    PORTIONS_MIN,
    ROUTE_SERVER,
    SERVICE_CANCEL_FEED,
    SERVICE_FEED,
)
from .kept import kept, remove as remove_kept
from .session import (
    STORE_UNREAD_ISSUE,
    UNCERTAIN_ISSUE,
    UNKNOWN_ISSUE,
    FeederSession,
    forget_token,
    hold_dropped,
    key_left,
    leftover_issue,
    open_feeder,
    token_left,
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
    """Version 2 is the first with two routes. A version 1 entry, from before
    them, is not carried over: it fails to migrate, and the owner deletes it
    and pairs again. A newer minor of version 2 grew only by what this build
    reads past, so it is read as it is and left at its minor; a newer major is
    refused before this is asked."""
    if entry.version == ENTRY_VERSION:
        return True
    if entry.version == 1:
        _LOGGER.error(
            "This Wellfed entry is from an earlier test build: delete it and pair the feeder again"
        )
    return False


async def async_setup_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
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

    def connected() -> None:
        entry.async_create_background_task(
            hass, _confirm_key_gone(hass, entry), "wellfed leftover key"
        )

    session.on_connected = connected
    session.start()

    # The first state names the role and what the feeder serves, which
    # decide the entities.
    await session.ready(READY_WAIT)
    if session.revoked or session.state is None:
        await session.stop()
        if session.revoked:
            key = "token_refused" if session.token_refused else "revoked"
            raise ConfigEntryAuthFailed(translation_domain=DOMAIN, translation_key=key)
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
    """A server entry's token goes with it; a direct key stays until the owner removes it,
    since a member may not remove its own. One the server could not revoke now is named
    in a repair, by its id, for the owner to revoke. A key an earlier route left on the
    feeder may still be listed: its repair outlives the entry, which can no longer check
    it, and the owner dismisses it once the key is removed. A hold on feeds goes with
    the entry, and a repair says so: a new entry for the feeder starts without it."""
    if entry.data.get(CONF_ROUTE) == ROUTE_SERVER:
        data = dict(entry.data)
        if not await hass.async_add_executor_job(forget_token, data):
            token_left(hass, data[CONF_URL], data[CONF_TOKEN_ID])
    if identity := entry.data.get(CONF_IDENTITY):
        feeder = bytes.fromhex(identity)
        store = kept(hass, entry.entry_id)
        await store.load()
        for key in store.leftover_keys:
            key_left(hass, feeder, key, LEFTOVER_KEY_LEFT)
        for issue in (UNCERTAIN_ISSUE, UNKNOWN_ISSUE, STORE_UNREAD_ISSUE):
            ir.async_delete_issue(hass, DOMAIN, f"{issue}_{identity}")
        if store.unread or any(p.released_ms is None for p in store.pending):
            hold_dropped(hass, feeder)
    await remove_kept(hass, entry.entry_id)


async def _confirm_key_gone(hass: HomeAssistant, entry: WellfedConfigEntry) -> None:
    """Clear each leftover key's repair once the feeder no longer lists the key, as the
    server reads it or, directly, as this Home Assistant's own key does; one no one
    can say about now stays as it is."""
    store = kept(hass, entry.entry_id)
    data = entry.data
    feeder = bytes.fromhex(data[CONF_IDENTITY])
    for key in store.leftover_keys:
        if data[CONF_ROUTE] == ROUTE_SERVER:
            listed = await hass.async_add_executor_job(
                bridge.server_client_listed,
                data[CONF_URL],
                data[CONF_TOKEN],
                data[CONF_FEEDER_ID],
                key,
            )
        else:
            listed = await hass.async_add_executor_job(
                bridge.client_listed,
                bytes.fromhex(data[CONF_SECRET]),
                feeder,
                key,
                data.get(CONF_ADDRESS),
            )
        if listed is False:
            store.remove_leftover_key(key)
            ir.async_delete_issue(hass, DOMAIN, leftover_issue(feeder, key))


async def async_remove_config_entry_device(
    hass: HomeAssistant, entry: WellfedConfigEntry, device: dr.DeviceEntry
) -> bool:
    """A device of this entry other than its feeder is stale, and may go."""
    return (DOMAIN, entry.data[CONF_IDENTITY]) not in device.identifiers
