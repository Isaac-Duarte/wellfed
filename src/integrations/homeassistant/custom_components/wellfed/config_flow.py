"""Pair Home Assistant with a feeder by an invite from a paired client.

A feeder is found by hand (an invite, and an address or mDNS) or by
zeroconf, which fills in the address. An invite is single-use, so each join
runs once: a failure asks for a new invite rather than retrying. A revoked
key asks for a new invite through reauth, under the same entry with a new
key. An owner invite also turns on owner mode, whose options name the
project's release keys.
"""

from __future__ import annotations

from collections.abc import Mapping
import re
from typing import Any

from homeassistant.config_entries import (
    ConfigEntry,
    ConfigFlow,
    ConfigFlowResult,
    OptionsFlow,
)
from homeassistant.core import callback
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo
import voluptuous as vol

import wellfed_homeassistant as bridge

from .const import (
    CLIENT_LABEL,
    CONF_ADDRESS,
    CONF_IDENTITY,
    CONF_INVITE,
    CONF_PROJECT_KEYS,
    CONF_ROLE,
    CONF_SECRET,
    DOMAIN,
)

INVITE_SCHEMA = vol.Schema({vol.Required(CONF_INVITE): str})
USER_SCHEMA = INVITE_SCHEMA.extend({vol.Optional(CONF_ADDRESS): str})

# A key as `wellfed invite` and the release tools print it.
KEY = re.compile(r"^[0-9a-fA-F]{64}$")


def _join(invite: str, address: str | None) -> tuple[bytes, bytes, bridge.Role]:
    """A new key joined with `invite`: its secret, the feeder, the role."""
    identity = bridge.invite_feeder(invite)
    secret = bridge.generate_secret()
    role = bridge.join(secret, invite, CLIENT_LABEL, address)
    return secret, identity, role


def _invite_feeder(invite: str) -> bytes | None:
    """The feeder `invite` names, or `None` for text that is not an invite."""
    try:
        return bridge.invite_feeder(invite)
    except bridge.WellfedError:
        return None


class WellfedConfigFlow(ConfigFlow, domain=DOMAIN):
    """Redeem an invite made with `wellfed invite` or the app."""

    VERSION = 1

    def __init__(self) -> None:
        self._address: str | None = None

    async def async_step_user(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        errors: dict[str, str] = {}
        if user_input is not None:
            invite = user_input[CONF_INVITE].strip()
            identity = _invite_feeder(invite)
            if identity is None:
                errors[CONF_INVITE] = "invalid_invite"
            else:
                await self.async_set_unique_id(identity.hex())
                self._abort_if_unique_id_configured()
                address = user_input.get(CONF_ADDRESS) or None
                result = await self._pair(invite, address, errors)
                if result is not None:
                    return result

        return self.async_show_form(step_id="user", data_schema=USER_SCHEMA, errors=errors)

    async def async_step_zeroconf(self, discovery_info: ZeroconfServiceInfo) -> ConfigFlowResult:
        identity = str(discovery_info.properties.get("id", ""))
        if not KEY.match(identity) or discovery_info.port is None:
            return self.async_abort(reason="not_a_feeder")
        host = discovery_info.ip_address
        address = f"[{host}]" if host.version == 6 else str(host)
        self._address = f"{address}:{discovery_info.port}"

        await self.async_set_unique_id(identity.lower())
        # A paired feeder that moved is found at its new address.
        self._abort_if_unique_id_configured(updates={CONF_ADDRESS: self._address})
        self.context["title_placeholders"] = {"name": discovery_info.name.split(".")[0]}
        return await self.async_step_zeroconf_confirm()

    async def async_step_zeroconf_confirm(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        errors: dict[str, str] = {}
        if user_input is not None:
            invite = user_input[CONF_INVITE].strip()
            identity = _invite_feeder(invite)
            if identity is None:
                errors[CONF_INVITE] = "invalid_invite"
            elif identity.hex() != self.unique_id:
                errors[CONF_INVITE] = "wrong_feeder"
            else:
                result = await self._pair(invite, self._address, errors)
                if result is not None:
                    return result

        return self.async_show_form(
            step_id="zeroconf_confirm",
            data_schema=INVITE_SCHEMA,
            errors=errors,
            description_placeholders={"address": self._address or ""},
        )

    async def _pair(
        self, invite: str, address: str | None, errors: dict[str, str]
    ) -> ConfigFlowResult | None:
        try:
            # An invite is single-use: this runs once, and a failure asks
            # the owner for a new invite rather than retrying.
            secret, identity, role = await self.hass.async_add_executor_job(_join, invite, address)
        except bridge.WellfedError:
            errors["base"] = "join_failed"
            return None
        return self.async_create_entry(
            title=f"Wellfed {identity.hex()[:8]}",
            data={
                CONF_SECRET: secret.hex(),
                CONF_IDENTITY: identity.hex(),
                CONF_ADDRESS: address,
                CONF_ROLE: role.key,
            },
            description="joined",
            description_placeholders={"role": role.key},
        )

    async def async_step_reauth(self, entry_data: Mapping[str, Any]) -> ConfigFlowResult:
        return await self.async_step_reauth_confirm()

    async def async_step_reauth_confirm(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """The feeder no longer knows this Home Assistant: join again, with a new key."""
        entry = self._get_reauth_entry()
        errors: dict[str, str] = {}
        if user_input is not None:
            invite = user_input[CONF_INVITE].strip()
            identity = _invite_feeder(invite)
            if identity is None:
                errors[CONF_INVITE] = "invalid_invite"
            elif identity.hex() != entry.unique_id:
                errors[CONF_INVITE] = "wrong_feeder"
            else:
                try:
                    secret, _, role = await self.hass.async_add_executor_job(
                        _join, invite, entry.data.get(CONF_ADDRESS)
                    )
                except bridge.WellfedError:
                    errors["base"] = "join_failed"
                else:
                    return self.async_update_reload_and_abort(
                        entry,
                        data_updates={CONF_SECRET: secret.hex(), CONF_ROLE: role.key},
                    )

        return self.async_show_form(
            step_id="reauth_confirm", data_schema=INVITE_SCHEMA, errors=errors
        )

    async def async_step_reconfigure(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Change where the feeder is found."""
        entry = self._get_reconfigure_entry()
        if user_input is not None:
            address = user_input.get(CONF_ADDRESS) or None
            return self.async_update_reload_and_abort(entry, data_updates={CONF_ADDRESS: address})

        current = entry.data.get(CONF_ADDRESS) or ""
        schema = vol.Schema({vol.Optional(CONF_ADDRESS, default=current): str})
        return self.async_show_form(step_id="reconfigure", data_schema=schema)

    @staticmethod
    @callback
    def async_get_options_flow(config_entry: ConfigEntry) -> OptionsFlow:
        return WellfedOptionsFlow()


class WellfedOptionsFlow(OptionsFlow):
    """Owner mode's options: the keys the project signs its releases with."""

    async def async_step_init(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if self.config_entry.data.get(CONF_ROLE) != bridge.Role.Owner.key:
            return self.async_abort(reason="owner_only")

        errors: dict[str, str] = {}
        if user_input is not None:
            keys = user_input.get(CONF_PROJECT_KEYS, "").replace(",", " ").split()
            if all(KEY.match(key) for key in keys):
                return self.async_create_entry(
                    data={CONF_PROJECT_KEYS: [key.lower() for key in keys]}
                )
            errors[CONF_PROJECT_KEYS] = "invalid_key"

        current = " ".join(self.config_entry.options.get(CONF_PROJECT_KEYS, []))
        schema = vol.Schema({vol.Optional(CONF_PROJECT_KEYS, default=current): str})
        return self.async_show_form(step_id="init", data_schema=schema, errors=errors)
