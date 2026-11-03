"""The Wellfed integration: a feeder paired directly, with no server."""

from __future__ import annotations

from pathlib import Path

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import ATTR_DEVICE_ID, Platform
from homeassistant.core import HomeAssistant, ServiceCall, ServiceResponse, SupportsResponse
from homeassistant.exceptions import (
    ConfigEntryAuthFailed,
    ConfigEntryNotReady,
    ServiceValidationError,
)
from homeassistant.helpers import config_validation as cv, device_registry as dr
from homeassistant.helpers.typing import ConfigType
import voluptuous as vol

import wellfed_homeassistant as bridge

from .const import (
    ATTR_FEED,
    ATTR_PATH,
    ATTR_PORTIONS,
    CONF_ADDRESS,
    CONF_IDENTITY,
    CONF_PROJECT_KEYS,
    CONF_SECRET,
    DOMAIN,
    PORTIONS_MAX,
    PORTIONS_MIN,
    SERVICE_ADD_RELEASE,
    SERVICE_CANCEL_FEED,
    SERVICE_FEED,
)
from .session import FeederSession

PLATFORMS = [
    Platform.BINARY_SENSOR,
    Platform.BUTTON,
    Platform.EVENT,
    Platform.NUMBER,
    Platform.SENSOR,
    Platform.SWITCH,
    Platform.UPDATE,
]

# How long setup waits for the feeder's first answer before it tries again
# later.
READY_WAIT = 15.0

type WellfedConfigEntry = ConfigEntry[FeederSession]

CONFIG_SCHEMA = cv.config_entry_only_config_schema(DOMAIN)

FEED_SCHEMA = vol.Schema(
    {
        vol.Required(ATTR_DEVICE_ID): cv.string,
        vol.Optional(ATTR_PORTIONS, default=1): vol.All(
            vol.Coerce(int), vol.Range(min=PORTIONS_MIN, max=PORTIONS_MAX)
        ),
    }
)
CANCEL_SCHEMA = vol.Schema(
    {
        vol.Required(ATTR_DEVICE_ID): cv.string,
        vol.Optional(ATTR_FEED): vol.All(vol.Coerce(int), vol.Range(min=0)),
    }
)
ADD_RELEASE_SCHEMA = vol.Schema(
    {
        vol.Required(ATTR_DEVICE_ID): cv.string,
        vol.Required(ATTR_PATH): cv.string,
    }
)


def release_folder(hass: HomeAssistant, entry: ConfigEntry) -> str:
    """Where an owner's releases for this feeder are kept."""
    return hass.config.path(DOMAIN, entry.data[CONF_IDENTITY])


def project_keys(entry: ConfigEntry) -> list[bytes]:
    """The project's release keys from owner mode's options."""
    return [bytes.fromhex(key) for key in entry.options.get(CONF_PROJECT_KEYS, [])]


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

    async def add_release(call: ServiceCall) -> ServiceResponse:
        entry = _entry(hass, call)
        session = entry.runtime_data
        if session.role != bridge.Role.Owner:
            raise ServiceValidationError(translation_domain=DOMAIN, translation_key="owner_only")
        path = call.data[ATTR_PATH]
        if not hass.config.is_allowed_path(path):
            raise ServiceValidationError(
                translation_domain=DOMAIN,
                translation_key="path_not_allowed",
                translation_placeholders={"path": path},
            )
        offer = await session.add_release(release_folder(hass, entry), project_keys(entry), path)
        return {"version": offer.version, "build": offer.build}

    hass.services.async_register(
        DOMAIN,
        SERVICE_FEED,
        feed,
        schema=FEED_SCHEMA,
        supports_response=SupportsResponse.OPTIONAL,
    )
    hass.services.async_register(DOMAIN, SERVICE_CANCEL_FEED, cancel_feed, schema=CANCEL_SCHEMA)
    hass.services.async_register(
        DOMAIN,
        SERVICE_ADD_RELEASE,
        add_release,
        schema=ADD_RELEASE_SCHEMA,
        supports_response=SupportsResponse.OPTIONAL,
    )
    return True


async def async_setup_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    session = FeederSession(
        hass,
        bytes.fromhex(entry.data[CONF_SECRET]),
        bytes.fromhex(entry.data[CONF_IDENTITY]),
        entry.data.get(CONF_ADDRESS),
    )
    session.on_revoked = lambda: entry.async_start_reauth(hass)
    session.start()

    # The first state names the role and what the feeder serves, which
    # decide the entities.
    await session.ready(READY_WAIT)
    if session.revoked or session.state is None:
        await hass.async_add_executor_job(session.stop)
        if session.revoked:
            raise ConfigEntryAuthFailed(translation_domain=DOMAIN, translation_key="revoked")
        raise ConfigEntryNotReady(translation_domain=DOMAIN, translation_key="not_ready")

    entry.runtime_data = session
    await hass.async_add_executor_job(
        lambda: Path(release_folder(hass, entry)).mkdir(parents=True, exist_ok=True)
    )
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    entry.async_on_unload(entry.add_update_listener(_options_changed))
    return True


async def _options_changed(hass: HomeAssistant, entry: WellfedConfigEntry) -> None:
    await hass.config_entries.async_reload(entry.entry_id)


async def async_unload_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    unloaded = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if unloaded:
        await hass.async_add_executor_job(entry.runtime_data.stop)
    return unloaded
