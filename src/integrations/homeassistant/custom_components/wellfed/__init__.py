"""The Wellfed integration: a feeder paired directly, with no server."""

from __future__ import annotations

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import ATTR_DEVICE_ID, Platform
from homeassistant.core import HomeAssistant, ServiceCall, SupportsResponse
from homeassistant.exceptions import ServiceValidationError
from homeassistant.helpers import config_validation as cv, device_registry as dr
import voluptuous as vol

from .const import (
    ATTR_PORTIONS,
    CONF_ADDRESS,
    CONF_IDENTITY,
    CONF_SECRET,
    DOMAIN,
    PORTIONS_MAX,
    PORTIONS_MIN,
    SERVICE_FEED,
)
from .session import FeederSession

PLATFORMS = [Platform.BINARY_SENSOR, Platform.BUTTON, Platform.SENSOR, Platform.SWITCH]

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


async def async_setup(hass: HomeAssistant, config: dict) -> bool:
    """Register the feed action, which names the feeder by its device."""

    async def feed(call: ServiceCall) -> dict[str, int]:
        device = dr.async_get(hass).async_get(call.data[ATTR_DEVICE_ID])
        entry = next(
            (
                entry
                for entry in hass.config_entries.async_loaded_entries(DOMAIN)
                if device is not None and entry.entry_id in device.config_entries
            ),
            None,
        )
        if entry is None:
            raise ServiceValidationError("That device is not a Wellfed feeder")
        feed_number = await entry.runtime_data.feed(call.data[ATTR_PORTIONS])
        return {"feed": feed_number}

    hass.services.async_register(
        DOMAIN,
        SERVICE_FEED,
        feed,
        schema=FEED_SCHEMA,
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
    session.start()
    entry.runtime_data = session
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: WellfedConfigEntry) -> bool:
    unloaded = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if unloaded:
        await hass.async_add_executor_job(entry.runtime_data.stop)
    return unloaded
