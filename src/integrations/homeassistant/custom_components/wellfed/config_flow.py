"""Set up a feeder directly, by a member invite from a paired client.

A feeder is found by hand (an invite, and an address or mDNS) or by
zeroconf, which fills in the address. An invite is single-use, so each join
runs once and is never sent again: a failure asks for a new invite rather
than retrying. A join whose answer was lost, and which the bridge could not
check, may have enrolled the key, so the flow keeps that key and asks the
feeder about it first when the owner tries again, before it spends another
invite. Home Assistant is a member and nothing more. An invite does not say
its role until it is redeemed, so the bridge redeems it, and a key enrolled
as anything but a member removes itself at once; the flow then asks for a
member invite.

The entry is one feeder, named by its identity key. Reauth asks for a new
invite; reconfigure changes the address.
"""

from __future__ import annotations

from collections.abc import Mapping
import logging
import re
from typing import Any

from homeassistant.config_entries import ConfigEntry, ConfigFlow, ConfigFlowResult
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
    ENTRY_MINOR_VERSION,
    ENTRY_VERSION,
)
from .session import key_left

_LOGGER = logging.getLogger(__name__)

INVITE_SCHEMA = probatio.Schema({probatio.Required(CONF_INVITE): str})
DIRECT_SCHEMA = INVITE_SCHEMA.extend({probatio.Optional(CONF_ADDRESS): str})

# The identity a feeder advertises: its key, 64 hex digits.
KEY = re.compile(r"^[0-9a-fA-F]{64}$")


class NotMember(Exception):
    """The invite was not a member's; `removed` says whether its key went again."""

    def __init__(self, removed: bool) -> None:
        super().__init__()
        self.removed = removed


def _member(redeemed: bridge.Redeemed) -> tuple[bytes, bytes]:
    """The key an invite enrolled, and its feeder, kept only as a member."""
    if redeemed.joined.role != bridge.Role.Member:
        raise NotMember(redeemed.joined.removed)
    return redeemed.secret, redeemed.feeder


def _invite_feeder(invite: str) -> bytes | None:
    """The feeder `invite` names, or `None` for text that is not an invite."""
    try:
        return bridge.invite_feeder(invite)
    except bridge.WellfedError:
        return None


def _unusable(entry: ConfigEntry) -> bool:
    """Whether `entry` cannot reach its feeder on this build: one from an early
    test build, or one with no client key, made for the owner's server, which
    Wellfed no longer has. The owner deletes it and pairs again."""
    return entry.version < ENTRY_VERSION or CONF_SECRET not in entry.data


def _data(secret: bytes, identity: bytes, address: str | None) -> dict[str, Any]:
    return {
        CONF_IDENTITY: identity.hex(),
        CONF_SECRET: secret.hex(),
        CONF_ADDRESS: address,
    }


class WellfedConfigFlow(ConfigFlow, domain=DOMAIN):
    """Set up one feeder, directly."""

    VERSION = ENTRY_VERSION
    MINOR_VERSION = ENTRY_MINOR_VERSION

    def __init__(self) -> None:
        self._address: str | None = None
        # A key an unchecked join may have enrolled, and its feeder: the
        # bridge checks it before the next invite for that feeder is spent.
        self._joining: bridge.HeldKey | None = None

    def async_remove(self) -> None:
        """The flow ended: a key an unchecked join may have enrolled is named in a
        repair, since nothing will check it now."""
        if (held := self._joining) is not None:
            self._joining = None
            key_left(self.hass, held.feeder, held.public_key())

    # Setup.

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
                joined = await self._redeem(invite, address, errors)
                if joined is not None:
                    secret, identity = joined
                    return self._entry(identity, _data(secret, identity, address))

        return self.async_show_form(step_id="user", data_schema=DIRECT_SCHEMA, errors=errors)

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
                joined = await self._redeem(invite, self._address, errors)
                if joined is not None:
                    secret, identity = joined
                    return self._entry(identity, _data(secret, identity, self._address))

        return self.async_show_form(
            step_id="zeroconf_confirm",
            data_schema=INVITE_SCHEMA,
            errors=errors,
            description_placeholders={"address": self._address or ""},
        )

    def _entry(self, identity: bytes, data: dict[str, Any]) -> ConfigFlowResult:
        return self.async_create_entry(title=f"Wellfed {identity.hex()[:8]}", data=data)

    # Reauth: a new invite.

    async def async_step_reauth(self, entry_data: Mapping[str, Any]) -> ConfigFlowResult:
        if _unusable(self._get_reauth_entry()):
            return self.async_abort(reason="pair_again")
        return await self.async_step_reauth_confirm()

    async def async_step_reauth_confirm(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """The feeder no longer knows this Home Assistant: join again, with a new key."""
        entry = self._get_reauth_entry()
        errors: dict[str, str] = {}
        if user_input is not None:
            joined = await self._rejoin(entry, user_input, entry.data.get(CONF_ADDRESS), errors)
            if joined is not None:
                secret, _ = joined
                return self.async_update_reload_and_abort(
                    entry, data_updates={CONF_SECRET: secret.hex()}
                )

        return self.async_show_form(
            step_id="reauth_confirm", data_schema=INVITE_SCHEMA, errors=errors
        )

    # Reconfigure: the address.

    async def async_step_reconfigure(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Change where the feeder is found."""
        entry = self._get_reconfigure_entry()
        if _unusable(entry):
            return self.async_abort(reason="pair_again")
        if user_input is not None:
            address = user_input.get(CONF_ADDRESS) or None
            return self.async_update_reload_and_abort(entry, data_updates={CONF_ADDRESS: address})

        current = entry.data.get(CONF_ADDRESS) or ""
        schema = probatio.Schema({probatio.Optional(CONF_ADDRESS, default=current): str})
        return self.async_show_form(step_id="reconfigure", data_schema=schema)

    # Shared steps.

    async def _redeem(
        self, invite: str, address: str | None, errors: dict[str, str]
    ) -> tuple[bytes, bytes] | None:
        """Join once with `invite`; on failure, the error to show and `None`."""
        try:
            # An invite is single-use: this runs once, and a failure asks the
            # owner for a new invite rather than retrying. The bridge checks a
            # key held from an unchecked join first, and says which is held.
            redeemed = await self.hass.async_add_executor_job(
                bridge.redeem, invite, CLIENT_LABEL, address, self._joining
            )
        except bridge.JoinFailed as failed:
            self._joining = failed.held
            errors["base"] = f"join_{failed.problem.key}"
            return None
        except bridge.WellfedError:
            errors["base"] = "join_failed"
            return None
        self._joining = redeemed.held
        try:
            return _member(redeemed)
        except NotMember as refused:
            errors["base"] = "not_member_invite" if refused.removed else "not_member_invite_kept"
        return None

    async def _rejoin(
        self,
        entry: ConfigEntry,
        user_input: dict[str, Any],
        address: str | None,
        errors: dict[str, str],
    ) -> tuple[bytes, bytes] | None:
        """Join with an invite for `entry`'s own feeder."""
        invite = user_input[CONF_INVITE].strip()
        identity = _invite_feeder(invite)
        if identity is None:
            errors[CONF_INVITE] = "invalid_invite"
        elif identity.hex() != entry.unique_id:
            errors[CONF_INVITE] = "wrong_feeder"
        else:
            return await self._redeem(invite, address, errors)
        return None
