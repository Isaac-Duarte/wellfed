"""Set up a feeder on one of two routes, chosen at setup.

- **Directly:** pair by a member invite from a paired client. A feeder is
  found by hand (an invite, and an address or mDNS) or by zeroconf, which
  fills in the address. An invite is single-use, so each join runs once: a
  failure asks for a new invite rather than retrying. Home Assistant is a
  member and nothing more. An invite does not say its role until it is
  redeemed, so the bridge redeems it, and a key enrolled as anything but a
  member removes itself at once; the flow then asks for a member invite.
- **Through the owner's server:** sign in with an account's name and
  password, which the bridge trades for a token of Home Assistant's own and
  then forgets, and pick one of the server's feeders. An account that may
  change nothing (a viewer) gets no token.

Either way the entry is one feeder, named by its identity key, so its
entities and their history are the same on both routes. Reauth asks for a
new invite (direct) or a new sign-in (server); reconfigure changes the
address, or moves the entry to the other route.
"""

from __future__ import annotations

from collections.abc import Mapping
import logging
import re
from typing import Any

from homeassistant.config_entries import ConfigEntry, ConfigFlow, ConfigFlowResult
from homeassistant.const import CONF_PASSWORD, CONF_USERNAME
from homeassistant.helpers import issue_registry as ir
from homeassistant.helpers.selector import (
    SelectOptionDict,
    SelectSelector,
    SelectSelectorConfig,
    TextSelector,
    TextSelectorConfig,
    TextSelectorType,
)
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo
import probatio

import wellfed_homeassistant as bridge

from .const import (
    CLIENT_LABEL,
    CONF_ADDRESS,
    CONF_FEEDER,
    CONF_FEEDER_ID,
    CONF_IDENTITY,
    CONF_INVITE,
    CONF_ROUTE,
    CONF_SECRET,
    CONF_TOKEN,
    CONF_TOKEN_ID,
    CONF_URL,
    DOMAIN,
    ROUTE_DIRECT,
    ROUTE_SERVER,
)
from .session import forget_token

_LOGGER = logging.getLogger(__name__)

INVITE_SCHEMA = probatio.Schema({probatio.Required(CONF_INVITE): str})
DIRECT_SCHEMA = INVITE_SCHEMA.extend({probatio.Optional(CONF_ADDRESS): str})
PASSWORD = TextSelector(TextSelectorConfig(type=TextSelectorType.PASSWORD))
SIGN_IN_SCHEMA = probatio.Schema(
    {probatio.Required(CONF_USERNAME): str, probatio.Required(CONF_PASSWORD): PASSWORD}
)

# The identity a feeder advertises: its key, 64 hex digits.
KEY = re.compile(r"^[0-9a-fA-F]{64}$")

# The repair raised when the key of an entry moved to the server route
# could not be removed from the feeder.
LEFTOVER_KEY_ISSUE = "leftover_key"


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


def _server_schema(url: str = "") -> probatio.Schema:
    return probatio.Schema({probatio.Required(CONF_URL, default=url): str}).extend(
        SIGN_IN_SCHEMA.schema
    )


def _direct_data(secret: bytes, identity: bytes, address: str | None) -> dict[str, Any]:
    return {
        CONF_ROUTE: ROUTE_DIRECT,
        CONF_IDENTITY: identity.hex(),
        CONF_SECRET: secret.hex(),
        CONF_ADDRESS: address,
    }


def _server_data(url: str, login: bridge.Login, feeder: bridge.ServerFeeder) -> dict[str, Any]:
    return {
        CONF_ROUTE: ROUTE_SERVER,
        CONF_IDENTITY: feeder.identity.hex(),
        CONF_URL: url,
        CONF_TOKEN: login.token,
        CONF_TOKEN_ID: login.token_id,
        CONF_FEEDER_ID: feeder.id,
    }


class WellfedConfigFlow(ConfigFlow, domain=DOMAIN):
    """Set up one feeder, directly or through the owner's server."""

    VERSION = 1
    MINOR_VERSION = 1

    def __init__(self) -> None:
        self._address: str | None = None
        # A sign-in this flow made, until its entry is written.
        self._url = ""
        self._login: bridge.Login | None = None
        self._feeders: list[bridge.ServerFeeder] = []

    # Setup.

    async def async_step_user(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        return self.async_show_menu(step_id="user", menu_options=["direct", "server"])

    async def async_step_direct(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
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
                    return self._entry(identity, _direct_data(secret, identity, address))

        return self.async_show_form(step_id="direct", data_schema=DIRECT_SCHEMA, errors=errors)

    async def async_step_server(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        errors: dict[str, str] = {}
        if user_input is not None:
            url = user_input[CONF_URL].strip().rstrip("/")
            if await self._sign_in(url, user_input, errors):
                return await self.async_step_server_feeder()

        return self.async_show_form(
            step_id="server", data_schema=_server_schema(self._url), errors=errors
        )

    async def async_step_server_feeder(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Pick one of the server's feeders not set up yet."""
        assert self._login is not None
        if user_input is not None:
            feeder = next(f for f in self._feeders if f.id == user_input[CONF_FEEDER])
            await self.async_set_unique_id(feeder.identity.hex())
            self._abort_if_unique_id_configured()
            return self._entry(feeder.identity, _server_data(self._url, self._login, feeder))

        configured = {entry.unique_id for entry in self._async_current_entries()}
        self._feeders = [f for f in self._feeders if f.identity.hex() not in configured]
        if not self._feeders:
            await self._drop_login()
            return self.async_abort(reason="no_feeders")
        options = [SelectOptionDict(value=f.id, label=f.name) for f in self._feeders]
        schema = probatio.Schema(
            {probatio.Required(CONF_FEEDER): SelectSelector(SelectSelectorConfig(options=options))}
        )
        return self.async_show_form(step_id="server_feeder", data_schema=schema)

    async def async_step_zeroconf(self, discovery_info: ZeroconfServiceInfo) -> ConfigFlowResult:
        identity = str(discovery_info.properties.get("id", ""))
        if not KEY.match(identity) or discovery_info.port is None:
            return self.async_abort(reason="not_a_feeder")
        host = discovery_info.ip_address
        address = f"[{host}]" if host.version == 6 else str(host)
        self._address = f"{address}:{discovery_info.port}"

        await self.async_set_unique_id(identity.lower())
        # A paired feeder that moved is found at its new address; one on the
        # server route is the server's to find.
        for entry in self._async_current_entries(include_ignore=False):
            if entry.unique_id == identity.lower() and entry.data.get(CONF_ROUTE) == ROUTE_SERVER:
                return self.async_abort(reason="already_configured")
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
                    return self._entry(identity, _direct_data(secret, identity, self._address))

        return self.async_show_form(
            step_id="zeroconf_confirm",
            data_schema=INVITE_SCHEMA,
            errors=errors,
            description_placeholders={"address": self._address or ""},
        )

    def _entry(self, identity: bytes, data: dict[str, Any]) -> ConfigFlowResult:
        return self.async_create_entry(title=f"Wellfed {identity.hex()[:8]}", data=data)

    # Reauth: a new invite directly, a new sign-in through the server.

    async def async_step_reauth(self, entry_data: Mapping[str, Any]) -> ConfigFlowResult:
        if entry_data[CONF_ROUTE] == ROUTE_SERVER:
            return await self.async_step_reauth_server()
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

    async def async_step_reauth_server(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """The server no longer takes the token: sign in again for a new one."""
        entry = self._get_reauth_entry()
        errors: dict[str, str] = {}
        if user_input is not None and await self._sign_in(entry.data[CONF_URL], user_input, errors):
            assert self._login is not None
            feeder = self._serves(entry)
            if feeder is not None:
                return self.async_update_reload_and_abort(
                    entry, data=_server_data(self._url, self._login, feeder)
                )
            await self._drop_login()
            errors["base"] = "not_on_server"

        return self.async_show_form(
            step_id="reauth_server",
            data_schema=SIGN_IN_SCHEMA,
            errors=errors,
            description_placeholders={CONF_URL: entry.data[CONF_URL]},
        )

    # Reconfigure: the address, or the other route.

    async def async_step_reconfigure(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        entry = self._get_reconfigure_entry()
        if entry.data[CONF_ROUTE] == ROUTE_SERVER:
            options = ["reconfigure_server", "reconfigure_direct"]
        else:
            options = ["reconfigure_address", "reconfigure_server"]
        return self.async_show_menu(step_id="reconfigure", menu_options=options)

    async def async_step_reconfigure_address(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Change where the feeder is found."""
        entry = self._get_reconfigure_entry()
        if user_input is not None:
            address = user_input.get(CONF_ADDRESS) or None
            return self.async_update_reload_and_abort(entry, data_updates={CONF_ADDRESS: address})

        current = entry.data.get(CONF_ADDRESS) or ""
        schema = probatio.Schema({probatio.Optional(CONF_ADDRESS, default=current): str})
        return self.async_show_form(step_id="reconfigure_address", data_schema=schema)

    async def async_step_reconfigure_server(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Reach the feeder through a server (another one, or as another account)."""
        entry = self._get_reconfigure_entry()
        errors: dict[str, str] = {}
        if user_input is not None:
            url = user_input[CONF_URL].strip().rstrip("/")
            if await self._sign_in(url, user_input, errors):
                assert self._login is not None
                feeder = self._serves(entry)
                if feeder is not None:
                    return await self._moved_to_server(entry, feeder)
                await self._drop_login()
                errors["base"] = "not_on_server"

        current = entry.data.get(CONF_URL, self._url)
        return self.async_show_form(
            step_id="reconfigure_server", data_schema=_server_schema(current), errors=errors
        )

    async def async_step_reconfigure_direct(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Reach the feeder directly, with a member invite, and drop the server's token."""
        entry = self._get_reconfigure_entry()
        errors: dict[str, str] = {}
        if user_input is not None:
            address = user_input.get(CONF_ADDRESS) or None
            joined = await self._rejoin(entry, user_input, address, errors)
            if joined is not None:
                secret, identity = joined
                await self.hass.async_add_executor_job(forget_token, dict(entry.data))
                return self.async_update_reload_and_abort(
                    entry, data=_direct_data(secret, identity, address)
                )

        return self.async_show_form(
            step_id="reconfigure_direct", data_schema=DIRECT_SCHEMA, errors=errors
        )

    async def _moved_to_server(
        self, entry: ConfigEntry, feeder: bridge.ServerFeeder
    ) -> ConfigFlowResult:
        """Write the server route over `entry`, and drop what the old route held."""
        assert self._login is not None
        old = dict(entry.data)
        if old[CONF_ROUTE] == ROUTE_SERVER:
            await self.hass.async_add_executor_job(forget_token, old)
        else:
            # A member may not remove its own key, so the server removes it
            # when it can (an owner's account, on a server the feeder made
            # an owner); otherwise the owner removes it in the app.
            public = bridge.public_key(bytes.fromhex(old[CONF_SECRET]))
            removed = await self.hass.async_add_executor_job(
                bridge.server_revoke_client, self._url, self._login.token, feeder.id, public
            )
            if not removed:
                ir.async_create_issue(
                    self.hass,
                    DOMAIN,
                    f"{LEFTOVER_KEY_ISSUE}_{feeder.identity.hex()}",
                    is_fixable=False,
                    is_persistent=True,
                    severity=ir.IssueSeverity.WARNING,
                    translation_key=LEFTOVER_KEY_ISSUE,
                    translation_placeholders={
                        "feeder": feeder.identity.hex()[:8],
                        "key": public.hex()[:8],
                    },
                )
        return self.async_update_reload_and_abort(
            entry, data=_server_data(self._url, self._login, feeder)
        )

    # Shared steps.

    async def _redeem(
        self, invite: str, address: str | None, errors: dict[str, str]
    ) -> tuple[bytes, bytes] | None:
        """Join once with `invite`; on failure, the error to show and `None`."""
        try:
            # An invite is single-use: this runs once, and a failure asks the
            # owner for a new invite rather than retrying.
            return await self.hass.async_add_executor_job(_join, invite, address)
        except NotMember as refused:
            errors["base"] = "owner_invite" if refused.removed else "owner_invite_kept"
        except bridge.WellfedError:
            errors["base"] = "join_failed"
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

    async def _sign_in(self, url: str, user_input: dict[str, Any], errors: dict[str, str]) -> bool:
        """Sign in for a token, and read the feeders it sees."""
        self._url = url
        try:
            self._login = await self.hass.async_add_executor_job(
                bridge.server_login,
                url,
                user_input[CONF_USERNAME],
                user_input[CONF_PASSWORD],
                CLIENT_LABEL,
            )
            self._feeders = await self.hass.async_add_executor_job(
                bridge.server_feeders, url, self._login.token
            )
        except bridge.ServerRefused as refused:
            errors["base"] = f"server_{refused.problem.key}"
            await self._drop_login()
            return False
        return True

    def _serves(self, entry: ConfigEntry) -> bridge.ServerFeeder | None:
        """The server's feeder that is `entry`'s, by its identity."""
        return next((f for f in self._feeders if f.identity.hex() == entry.unique_id), None)

    async def _drop_login(self) -> None:
        """Revoke a token this flow made and will not keep."""
        login, self._login = self._login, None
        if login is not None:
            await self.hass.async_add_executor_job(
                forget_token,
                {CONF_URL: self._url, CONF_TOKEN: login.token, CONF_TOKEN_ID: login.token_id},
            )
