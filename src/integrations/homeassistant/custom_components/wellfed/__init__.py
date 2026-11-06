"""The Wellfed integration: a feeder paired directly, with no server."""

from __future__ import annotations

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
import probatio

from .const import (
    ATTR_FEED,
    ATTR_PORTIONS,
    CONF_ADDRESS,
    CONF_IDENTITY,
    CONF_SECRET,
    DOMAIN,
    PORTIONS_MAX,
    PORTIONS_MIN,
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
]

# How long setup waits for the feeder's first answer before it tries again
# later.
READY_WAIT = 15.0

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
    if session.revoked or session.other_build or session.state is None:
        await hass.async_add_executor_job(session.stop)
        if session.revoked:
            raise ConfigEntryAuthFailed(translation_domain=DOMAIN, translation_key="revoked")
        # Home Assistant tries again on its own, backing off: once the
        # feeder is updated, the entry comes back with no reload.
        if session.other_build:
            raise ConfigEntryNotReady(translation_domain=DOMAIN, translation_key="other_build")
        raise ConfigEntryNotReady(translation_domain=DOMAIN, translation_key="not_ready")

    # A feeder found on another build mid-session takes the same path:
    # set up again, which waits, retrying, for its update.
    session.on_other_build = lambda: hass.config_entries.async_schedule_reload(entry.entry_id)
    entry.runtime_data = session
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    unloaded = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if unloaded:
        await hass.async_add_executor_job(entry.runtime_data.stop)
    return unloaded


async def async_remove_config_entry_device(
    hass: HomeAssistant, entry: WellfedConfigEntry, device: dr.DeviceEntry
) -> bool:
    """A device of this entry other than its feeder is stale, and may go."""
    return (DOMAIN, entry.data[CONF_IDENTITY]) not in device.identifiers
