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
    CONF_FEEDER_ID,
    CONF_IDENTITY,
    CONF_ROUTE,
    CONF_TOKEN,
    CONF_TOKEN_ID,
    CONF_URL,
    DOMAIN,
    LEFTOVER_KEY_ISSUE,
    PORTIONS_MAX,
    PORTIONS_MIN,
    ROUTE_SERVER,
    SERVICE_CANCEL_FEED,
    SERVICE_FEED,
)
from .kept import kept, remove as remove_kept
from .session import FeederSession, forget_token, open_feeder, token_left

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
    and pairs again. An entry a newer integration wrote is refused too."""
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
            open_feeder, entry.data, store.pending, store.answered
        )
    except bridge.WellfedError as error:
        raise ConfigEntryError(translation_domain=DOMAIN, translation_key="bad_entry") from error
    session = FeederSession(hass, feeder, bytes.fromhex(entry.data[CONF_IDENTITY]), store)
    session.on_revoked = lambda: entry.async_start_reauth(hass)
    if entry.data[CONF_ROUTE] == ROUTE_SERVER:

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
        await hass.async_add_executor_job(session.stop)
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
        await hass.async_add_executor_job(entry.runtime_data.stop)
    return unloaded


async def async_remove_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> None:
    """A server entry's token goes with it; a direct key stays until the owner removes it,
    since a member may not remove its own. One the server could not revoke now is named
    in a repair, by its id, for the owner to revoke."""
    if entry.data.get(CONF_ROUTE) == ROUTE_SERVER:
        data = dict(entry.data)
        if not await hass.async_add_executor_job(forget_token, data):
            token_left(hass, data[CONF_URL], data[CONF_TOKEN_ID])
    if identity := entry.data.get(CONF_IDENTITY):
        ir.async_delete_issue(hass, DOMAIN, f"{LEFTOVER_KEY_ISSUE}_{identity}")
    await remove_kept(hass, entry.entry_id)


async def _confirm_key_gone(hass: HomeAssistant, entry: WellfedConfigEntry) -> None:
    """Clear the leftover key's repair once the server says the feeder no longer
    lists the key; one the server cannot say about stays as it is."""
    store = kept(hass, entry.entry_id)
    key = store.leftover_key
    if key is None:
        return
    data = entry.data
    listed = await hass.async_add_executor_job(
        bridge.server_client_listed, data[CONF_URL], data[CONF_TOKEN], data[CONF_FEEDER_ID], key
    )
    if listed is False:
        store.set_leftover_key(None)
        ir.async_delete_issue(hass, DOMAIN, f"{LEFTOVER_KEY_ISSUE}_{data[CONF_IDENTITY]}")


async def async_remove_config_entry_device(
    hass: HomeAssistant, entry: WellfedConfigEntry, device: dr.DeviceEntry
) -> bool:
    """A device of this entry other than its feeder is stale, and may go."""
    return (DOMAIN, entry.data[CONF_IDENTITY]) not in device.identifiers
