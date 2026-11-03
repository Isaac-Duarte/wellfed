"""Pair Home Assistant with a feeder by a member invite from a paired client.

A feeder is found by hand (an invite, and an address or mDNS) or by
zeroconf, which fills in the address. An invite is single-use, so each join
runs once: a failure asks for a new invite rather than retrying. A revoked
key asks for a new invite through reauth, under the same entry with a new
key.

Home Assistant is a member and nothing more. An invite does not say its
role until it is redeemed, so the bridge redeems it, and a key enrolled as
anything but a member removes itself at once; the flow then asks for a
member invite.
"""

from __future__ import annotations

from collections.abc import Mapping
import re
from typing import Any

from homeassistant.config_entries import ConfigFlow, ConfigFlowResult
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo
import probatio

import wellfed_homeassistant as bridge

from .const import (
    CLIENT_LABEL,
    CONF_ADDRESS,
    CONF_IDENTITY,
    CONF_INVITE,
    CONF_SECRET,
    DOMAIN,
)

INVITE_SCHEMA = probatio.Schema({probatio.Required(CONF_INVITE): str})
USER_SCHEMA = INVITE_SCHEMA.extend({probatio.Optional(CONF_ADDRESS): str})

# The identity a feeder advertises: its key, 64 hex digits.
KEY = re.compile(r"^[0-9a-fA-F]{64}$")


class NotMember(Exception):
    """The invite was not a member's; `removed` says whether its key went again."""

    def __init__(self, removed: bool) -> None:
        super().__init__()
        self.removed = removed


def _join(invite: str, address: str | None) -> tuple[bytes, bytes]:
    """A new key joined as a member with `invite`: its secret and the feeder."""
    identity = bridge.invite_feeder(invite)
    secret = bridge.generate_secret()
    joined = bridge.join(secret, invite, CLIENT_LABEL, address)
    if joined.role != bridge.Role.Member:
        raise NotMember(joined.removed)
    return secret, identity


def _invite_feeder(invite: str) -> bytes | None:
    """The feeder `invite` names, or `None` for text that is not an invite."""
    try:
        return bridge.invite_feeder(invite)
    except bridge.WellfedError:
        return None


async def _redeem(
    flow: ConfigFlow, invite: str, address: str | None, errors: dict[str, str]
) -> tuple[bytes, bytes] | None:
    """Join once with `invite`; on failure, the error to show and `None`."""
    try:
        # An invite is single-use: this runs once, and a failure asks the
        # owner for a new invite rather than retrying.
        return await flow.hass.async_add_executor_job(_join, invite, address)
    except NotMember as refused:
        errors["base"] = "owner_invite" if refused.removed else "owner_invite_kept"
    except bridge.WellfedError:
        errors["base"] = "join_failed"
    return None


class WellfedConfigFlow(ConfigFlow, domain=DOMAIN):
    """Redeem a member invite made with `wellfed invite` or the app."""

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
                joined = await _redeem(self, invite, address, errors)
                if joined is not None:
                    return self._entry(*joined, address)

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
                joined = await _redeem(self, invite, self._address, errors)
                if joined is not None:
                    return self._entry(*joined, self._address)

        return self.async_show_form(
            step_id="zeroconf_confirm",
            data_schema=INVITE_SCHEMA,
            errors=errors,
            description_placeholders={"address": self._address or ""},
        )

    def _entry(self, secret: bytes, identity: bytes, address: str | None) -> ConfigFlowResult:
        return self.async_create_entry(
            title=f"Wellfed {identity.hex()[:8]}",
            data={
                CONF_SECRET: secret.hex(),
                CONF_IDENTITY: identity.hex(),
                CONF_ADDRESS: address,
            },
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
                joined = await _redeem(self, invite, entry.data.get(CONF_ADDRESS), errors)
                if joined is not None:
                    secret, _ = joined
                    return self.async_update_reload_and_abort(
                        entry, data_updates={CONF_SECRET: secret.hex()}
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
        schema = probatio.Schema({probatio.Optional(CONF_ADDRESS, default=current): str})
        return self.async_show_form(step_id="reconfigure", data_schema=schema)
