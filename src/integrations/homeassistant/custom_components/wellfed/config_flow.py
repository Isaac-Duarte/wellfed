"""Pair Home Assistant with a feeder by an invite from a paired client."""

from __future__ import annotations

from typing import Any

from homeassistant.config_entries import ConfigFlow, ConfigFlowResult
import voluptuous as vol

import wellfed_homeassistant as bridge

from .const import CLIENT_LABEL, CONF_ADDRESS, CONF_IDENTITY, CONF_INVITE, CONF_SECRET, DOMAIN

SCHEMA = vol.Schema(
    {
        vol.Required(CONF_INVITE): str,
        vol.Optional(CONF_ADDRESS): str,
    }
)


def _join(invite: str, address: str | None) -> tuple[bytes, bytes, str]:
    """A new key joined with `invite`: its secret, the feeder, the role."""
    identity = bridge.invite_feeder(invite)
    secret = bridge.generate_secret()
    role = bridge.join(secret, invite, CLIENT_LABEL, address)
    return secret, identity, role


class WellfedConfigFlow(ConfigFlow, domain=DOMAIN):
    """Redeem an invite made with `wellfed invite` or the app."""

    VERSION = 1

    async def async_step_user(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        errors: dict[str, str] = {}
        if user_input is not None:
            invite = user_input[CONF_INVITE].strip()
            address = user_input.get(CONF_ADDRESS) or None
            try:
                identity = bridge.invite_feeder(invite)
            except bridge.WellfedError:
                errors[CONF_INVITE] = "invalid_invite"
            else:
                await self.async_set_unique_id(identity.hex())
                self._abort_if_unique_id_configured()
                try:
                    # An invite is single-use: this runs once, and a failure
                    # asks the owner for a new invite rather than retrying.
                    secret, identity, role = await self.hass.async_add_executor_job(
                        _join, invite, address
                    )
                except bridge.WellfedError:
                    errors["base"] = "join_failed"
                else:
                    return self.async_create_entry(
                        title=f"Wellfed {identity.hex()[:8]}",
                        data={
                            CONF_SECRET: secret.hex(),
                            CONF_IDENTITY: identity.hex(),
                            CONF_ADDRESS: address,
                        },
                        description_placeholders={"role": role},
                    )

        return self.async_show_form(step_id="user", data_schema=SCHEMA, errors=errors)
