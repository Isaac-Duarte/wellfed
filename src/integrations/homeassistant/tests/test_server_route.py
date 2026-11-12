"""The server route against the owner's server in front of a simulated
feeder (simulation, not hardware): setup by sign-in, a feed through the
server, the same entities on both routes when an entry switches with no
reauth on the way, reauth after a revoked token, setup's refusals, a plain
`http://` address confirmed first, and a token revoked whenever a flow
ends without keeping it."""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Mapping
from typing import Any

from homeassistant.config_entries import (
    SOURCE_REAUTH,
    SOURCE_RECONFIGURE,
    SOURCE_USER,
    ConfigEntry,
)
from homeassistant.const import ATTR_ENTITY_ID, CONF_PASSWORD, CONF_USERNAME
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import entity_registry as er, issue_registry as ir
import pytest

from custom_components.wellfed.const import (
    CONF_ADDRESS,
    CONF_FEEDER,
    CONF_INVITE,
    CONF_ROUTE,
    CONF_SECRET,
    CONF_TOKEN,
    CONF_TOKEN_ID,
    CONF_URL,
    DOMAIN,
    EVENT_FEEDER,
    ROUTE_DIRECT,
    ROUTE_SERVER,
)
from custom_components.wellfed.session import leftover_issue
import wellfed_homeassistant as bridge

from .conftest import OWNER, OWNER_PASSWORD, Server, Simulator

WAIT = 30


async def _until(hass: HomeAssistant, condition: Callable[[], bool]) -> None:
    async with asyncio.timeout(WAIT):
        while not condition():
            await asyncio.sleep(0.1)
            await hass.async_block_till_done()


def _sign_in(server: Server, password: str = OWNER_PASSWORD) -> dict[str, str]:
    return {CONF_URL: server.url, CONF_USERNAME: OWNER, CONF_PASSWORD: password}


async def _server_entry(hass: HomeAssistant, server: Server) -> ConfigEntry:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    assert result["type"] is FlowResultType.MENU
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": "server"}
    )
    result = await hass.config_entries.flow.async_configure(result["flow_id"], _sign_in(server))
    assert result["step_id"] == "server_feeder"
    feeder = server.call("GET", "/feeders")[0]["id"]
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_FEEDER: feeder}
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    entry: ConfigEntry = result["result"]
    await hass.async_block_till_done()
    await _until(
        hass, lambda: entry.runtime_data.connected and entry.runtime_data.state is not None
    )
    return entry


async def _direct_entry(hass: HomeAssistant, simulator: Simulator) -> ConfigEntry:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": "direct"}
    )
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_INVITE: simulator.invite(), CONF_ADDRESS: simulator.address}
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    entry: ConfigEntry = result["result"]
    await hass.async_block_till_done()
    await _until(
        hass, lambda: entry.runtime_data.connected and entry.runtime_data.state is not None
    )
    return entry


def _reauth_flows(hass: HomeAssistant) -> list[str]:
    return [
        flow["flow_id"]
        for flow in hass.config_entries.flow.async_progress_by_handler(DOMAIN)
        if flow["context"].get("source") == SOURCE_REAUTH
    ]


async def _at_picker(hass: HomeAssistant, server: Server) -> str:
    """A setup flow signed in and showing the server's feeders: its id."""
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": "server"}
    )
    result = await hass.config_entries.flow.async_configure(result["flow_id"], _sign_in(server))
    assert result["step_id"] == "server_feeder"
    return str(result["flow_id"])


def _unique_ids(hass: HomeAssistant, entry: ConfigEntry) -> set[str]:
    registry = er.async_get(hass)
    return {e.unique_id for e in er.async_entries_for_config_entry(registry, entry.entry_id)}


async def _reconfigure(
    hass: HomeAssistant, entry: ConfigEntry, step: str, user_input: Mapping[str, Any]
) -> None:
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_RECONFIGURE, "entry_id": entry.entry_id}
    )
    assert result["type"] is FlowResultType.MENU
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": step}
    )
    result = await hass.config_entries.flow.async_configure(result["flow_id"], dict(user_input))
    assert result["type"] is FlowResultType.ABORT, result.get("errors")
    assert result["reason"] == "reconfigure_successful"
    await hass.async_block_till_done()
    await _until(
        hass, lambda: entry.runtime_data.connected and entry.runtime_data.state is not None
    )


async def test_setup_signs_in_and_a_feed_goes_through_the_server(
    hass: HomeAssistant, simulator: Simulator, server: Server
) -> None:
    server.adopt(simulator)
    entry = await _server_entry(hass, server)
    assert entry.data[CONF_ROUTE] == ROUTE_SERVER
    assert entry.unique_id == bridge.invite_feeder(simulator.invite()).hex()
    # A token of its own, and no password or session kept.
    tokens = server.call("GET", "/tokens")
    assert [token["name"] for token in tokens] == ["home-assistant"]
    assert CONF_PASSWORD not in entry.data

    fed: list[Mapping[str, Any]] = []
    hass.bus.async_listen(
        EVENT_FEEDER,
        lambda event: fed.append(event.data) if event.data["kind"] == "fed" else None,
    )
    short = entry.unique_id[:8]
    await hass.services.async_call(
        "button", "press", {ATTR_ENTITY_ID: f"button.feeder_{short}_feed"}, blocking=True
    )
    await _until(hass, lambda: len(fed) == 1)
    assert fed[0]["completed"] == 1
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_switching_routes_keeps_the_entities_and_drops_the_old_route(
    hass: HomeAssistant, simulator: Simulator, server: Server
) -> None:
    server.adopt(simulator)
    entry = await _direct_entry(hass, simulator)
    direct = _unique_ids(hass, entry)
    old_key = bridge.public_key(bytes.fromhex(entry.data[CONF_SECRET])).hex()

    await _reconfigure(hass, entry, "reconfigure_server", _sign_in(server))
    assert entry.data[CONF_ROUTE] == ROUTE_SERVER
    assert CONF_SECRET not in entry.data
    assert _unique_ids(hass, entry) == direct
    # The server, an owner on the feeder, removed the key held directly.
    assert old_key not in simulator.owner("clients")
    assert entry.unique_id is not None
    leftover = leftover_issue(bytes.fromhex(entry.unique_id), bytes.fromhex(old_key))
    assert ir.async_get(hass).async_get_issue(DOMAIN, leftover) is None
    # The direct session stopped first, so its key's removal started no reauth.
    await asyncio.sleep(3)
    await hass.async_block_till_done()
    assert _reauth_flows(hass) == []

    await _reconfigure(
        hass,
        entry,
        "reconfigure_direct",
        {CONF_INVITE: simulator.invite(), CONF_ADDRESS: simulator.address},
    )
    assert entry.data[CONF_ROUTE] == ROUTE_DIRECT
    assert CONF_TOKEN not in entry.data
    assert _unique_ids(hass, entry) == direct
    # The server's token for Home Assistant is revoked, after its session
    # stopped, so the refusal started no reauth.
    assert server.call("GET", "/tokens") == []
    await asyncio.sleep(3)
    await hass.async_block_till_done()
    assert _reauth_flows(hass) == []

    # Back to the server, then server to server: the old token goes after
    # its session stopped too.
    await _reconfigure(hass, entry, "reconfigure_server", _sign_in(server))
    await _reconfigure(hass, entry, "reconfigure_server", _sign_in(server))
    await asyncio.sleep(3)
    await hass.async_block_till_done()
    assert _reauth_flows(hass) == []
    assert len(server.call("GET", "/tokens")) == 1
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_key_left_on_the_feeder_keeps_its_repair_back_on_the_direct_route(
    hass: HomeAssistant, simulator: Simulator, server: Server, monkeypatch: pytest.MonkeyPatch
) -> None:
    server.adopt(simulator)
    entry = await _direct_entry(hass, simulator)
    old_key = bridge.public_key(bytes.fromhex(entry.data[CONF_SECRET])).hex()
    assert entry.unique_id is not None
    leftover = leftover_issue(bytes.fromhex(entry.unique_id), bytes.fromhex(old_key))

    # The server could not remove the direct key (as for a member's account).
    monkeypatch.setattr(bridge, "server_revoke_client", lambda *_: False)
    await _reconfigure(hass, entry, "reconfigure_server", _sign_in(server))
    monkeypatch.undo()
    assert ir.async_get(hass).async_get_issue(DOMAIN, leftover) is not None

    # Back on the direct route, with a new key: the old one is still on the
    # feeder, and so is its repair.
    await _reconfigure(
        hass,
        entry,
        "reconfigure_direct",
        {CONF_INVITE: simulator.invite(), CONF_ADDRESS: simulator.address},
    )
    assert old_key in simulator.owner("clients")
    await asyncio.sleep(1)
    await hass.async_block_till_done()
    assert ir.async_get(hass).async_get_issue(DOMAIN, leftover) is not None

    # Once the owner removes it, the direct session's own key sees it gone.
    simulator.owner("revoke", old_key[:16])
    assert await hass.config_entries.async_reload(entry.entry_id)
    await _until(hass, lambda: ir.async_get(hass).async_get_issue(DOMAIN, leftover) is None)
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_revoked_token_asks_to_sign_in_again(
    hass: HomeAssistant, simulator: Simulator, server: Server
) -> None:
    server.adopt(simulator)
    entry = await _server_entry(hass, server)
    old_token = entry.data[CONF_TOKEN_ID]
    server.call("DELETE", f"/tokens/{old_token}")

    # The session finds out at its next read, within 30 seconds; a feed
    # finds out at once, and is refused unsent.
    with pytest.raises(HomeAssistantError) as raised:
        await entry.runtime_data.feed(1)
    assert raised.value.translation_key == "not_sent_stopped"

    def reauth_flows() -> list[str]:
        return _reauth_flows(hass)

    await _until(hass, lambda: len(reauth_flows()) == 1)
    assert entry.runtime_data.token_refused

    result = await hass.config_entries.flow.async_configure(
        reauth_flows()[0], {CONF_USERNAME: OWNER, CONF_PASSWORD: "not the password"}
    )
    assert result["errors"] == {"base": "server_bad_credentials"}
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_USERNAME: OWNER, CONF_PASSWORD: OWNER_PASSWORD}
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "reauth_successful"
    assert entry.data[CONF_TOKEN_ID] != old_token
    await hass.async_block_till_done()
    await _until(
        hass, lambda: entry.runtime_data.connected and entry.runtime_data.state is not None
    )
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_setup_refuses_a_viewer_a_wrong_address_and_a_server_with_no_feeder(
    hass: HomeAssistant, server: Server
) -> None:
    server.call(
        "POST",
        "/users",
        {"name": "vic", "password": "a viewer's password", "role": "viewer"},
    )

    async def signed_in(user_input: dict[str, str]) -> Any:
        result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
        result = await hass.config_entries.flow.async_configure(
            result["flow_id"], {"next_step_id": "server"}
        )
        return await hass.config_entries.flow.async_configure(result["flow_id"], user_input)

    viewer = await signed_in(
        {CONF_URL: server.url, CONF_USERNAME: "vic", CONF_PASSWORD: "a viewer's password"}
    )
    assert viewer["errors"] == {"base": "server_forbidden"}

    nothing = await signed_in({**_sign_in(server), CONF_URL: "http://127.0.0.1:9"})
    assert nothing["errors"] == {"base": "server_unreachable"}

    empty = await signed_in(_sign_in(server))
    assert empty["type"] is FlowResultType.ABORT
    assert empty["reason"] == "no_feeders"
    # Neither left a token behind.
    assert server.call("GET", "/tokens") == []


async def test_a_flow_closed_or_refused_at_the_feeder_picker_revokes_its_token(
    hass: HomeAssistant, simulator: Simulator, server: Server
) -> None:
    server.adopt(simulator)
    closed = await _at_picker(hass, server)
    hass.config_entries.flow.async_abort(closed)
    await _until(hass, lambda: server.call("GET", "/tokens") == [])

    # Two flows reach the picker; the first sets the feeder up, and the
    # second, picking it too, is refused with its token revoked.
    first = await _at_picker(hass, server)
    second = await _at_picker(hass, server)
    feeder = server.call("GET", "/feeders")[0]["id"]
    result = await hass.config_entries.flow.async_configure(first, {CONF_FEEDER: feeder})
    assert result["type"] is FlowResultType.CREATE_ENTRY
    result = await hass.config_entries.flow.async_configure(second, {CONF_FEEDER: feeder})
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "already_configured"
    await hass.async_block_till_done()
    entry: ConfigEntry = hass.config_entries.async_entries(DOMAIN)[0]
    tokens = server.call("GET", "/tokens")
    assert [token["id"] for token in tokens] == [entry.data[CONF_TOKEN_ID]]
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_a_plain_http_address_off_this_host_is_confirmed_first(
    hass: HomeAssistant, server: Server
) -> None:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": "server"}
    )
    away = {**_sign_in(server), CONF_URL: "http://wellfed.invalid:9"}
    result = await hass.config_entries.flow.async_configure(result["flow_id"], away)
    assert result["step_id"] == "plain_http"
    assert result["description_placeholders"] == {CONF_URL: "http://wellfed.invalid:9"}

    # Confirmed, the sign-in goes on with what was typed.
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    assert result["step_id"] == "server"
    assert result["errors"] == {"base": "server_unreachable"}
    hass.config_entries.flow.async_abort(result["flow_id"])

    # This host's own address on plain `http://` needs no confirmation.
    assert server.url.startswith("http://127.0.0.1")
