"""The config flows against a simulated feeder: zeroconf, reauth after a
revocation, reconfigure and owner mode's options."""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from ipaddress import ip_address
import subprocess

from homeassistant.config_entries import (
    SOURCE_REAUTH,
    SOURCE_RECONFIGURE,
    SOURCE_USER,
    SOURCE_ZEROCONF,
    ConfigEntry,
)
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo

from custom_components.wellfed.const import (
    CONF_ADDRESS,
    CONF_INVITE,
    CONF_PROJECT_KEYS,
    CONF_ROLE,
    CONF_SECRET,
    DOMAIN,
)
import wellfed_homeassistant as bridge

from .conftest import Simulator

WAIT = 30


async def _until(hass: HomeAssistant, condition: Callable[[], bool]) -> None:
    async with asyncio.timeout(WAIT):
        while not condition():
            await asyncio.sleep(0.1)
            await hass.async_block_till_done()


def _discovery(simulator: Simulator) -> ZeroconfServiceInfo:
    """What zeroconf hears from the simulator, as the feeder advertises it."""
    identity = bridge.invite_feeder(simulator.invite()).hex()
    host, port = simulator.address.rsplit(":", 1)
    instance = f"wellfed-{identity[:8]}"
    return ZeroconfServiceInfo(
        ip_address=ip_address(host),
        ip_addresses=[ip_address(host)],
        port=int(port),
        hostname=f"{instance}.local.",
        type="_wellfed._tcp.local.",
        name=f"{instance}._wellfed._tcp.local.",
        properties={"v": "1", "id": identity},
    )


async def _pair(hass: HomeAssistant, simulator: Simulator, role: str = "member") -> ConfigEntry:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"],
        {CONF_INVITE: simulator.invite(role), CONF_ADDRESS: simulator.address},
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    entry: ConfigEntry = result["result"]
    await hass.async_block_till_done()
    await _until(hass, lambda: entry.runtime_data.state is not None)
    return entry


async def test_zeroconf_fills_in_the_address_and_asks_for_the_invite(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    discovery = _discovery(simulator)
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_ZEROCONF}, data=discovery
    )
    assert result["type"] is FlowResultType.FORM
    assert result["step_id"] == "zeroconf_confirm"

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_INVITE: simulator.invite()}
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    entry = result["result"]
    assert entry.unique_id == discovery.properties["id"]
    assert entry.data[CONF_ADDRESS] == simulator.address
    assert entry.data[CONF_ROLE] == "member"
    await hass.async_block_till_done()
    await _until(hass, lambda: entry.runtime_data.state is not None)

    # The same feeder heard again is already paired.
    again = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_ZEROCONF}, data=discovery
    )
    assert again["type"] is FlowResultType.ABORT
    assert again["reason"] == "already_configured"
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_zeroconf_refuses_an_invite_for_another_feeder(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    discovery = _discovery(simulator)
    discovery.properties["id"] = "ab" * 32
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_ZEROCONF}, data=discovery
    )
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_INVITE: simulator.invite()}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {CONF_INVITE: "wrong_feeder"}


async def test_a_device_without_a_feeder_id_is_not_a_feeder(hass: HomeAssistant) -> None:
    discovery = ZeroconfServiceInfo(
        ip_address=ip_address("192.0.2.9"),
        ip_addresses=[ip_address("192.0.2.9")],
        port=4242,
        hostname="other.local.",
        type="_wellfed._tcp.local.",
        name="other._wellfed._tcp.local.",
        properties={},
    )
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_ZEROCONF}, data=discovery
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "not_a_feeder"


async def test_a_revoked_key_asks_for_a_new_invite_and_pairs_again(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    entry = await _pair(hass, simulator)
    old_secret = entry.data[CONF_SECRET]
    public = bridge.public_key(bytes.fromhex(old_secret)).hex()

    subprocess.run(
        [simulator.cli, "revoke", "--keys", simulator.keys, "--address", simulator.address, public],
        check=True,
        capture_output=True,
        timeout=30,
    )

    def reauth_flows() -> list[str]:
        return [
            flow["flow_id"]
            for flow in hass.config_entries.flow.async_progress_by_handler(DOMAIN)
            if flow["context"].get("source") == SOURCE_REAUTH
        ]

    await _until(hass, lambda: len(reauth_flows()) == 1)
    assert entry.runtime_data.revoked

    result = await hass.config_entries.flow.async_configure(
        reauth_flows()[0], {CONF_INVITE: simulator.invite()}
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "reauth_successful"
    assert entry.data[CONF_SECRET] != old_secret
    await hass.async_block_till_done()
    await _until(
        hass,
        lambda: entry.runtime_data.connected and entry.runtime_data.state is not None,
    )
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_reconfigure_changes_the_address(hass: HomeAssistant, simulator: Simulator) -> None:
    entry = await _pair(hass, simulator)
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_RECONFIGURE, "entry_id": entry.entry_id}
    )
    assert result["step_id"] == "reconfigure"

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_ADDRESS: simulator.address}
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "reconfigure_successful"
    assert entry.data[CONF_ADDRESS] == simulator.address
    await hass.async_block_till_done()
    assert await hass.config_entries.async_unload(entry.entry_id)


async def test_release_keys_are_an_owner_s_option(
    hass: HomeAssistant, simulator: Simulator
) -> None:
    member = await _pair(hass, simulator)
    result = await hass.config_entries.options.async_init(member.entry_id)
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "owner_only"
    assert await hass.config_entries.async_remove(member.entry_id)

    owner = await _pair(hass, simulator, role="owner")
    assert owner.data[CONF_ROLE] == "owner"
    result = await hass.config_entries.options.async_init(owner.entry_id)
    result = await hass.config_entries.options.async_configure(
        result["flow_id"], {CONF_PROJECT_KEYS: "not-a-key"}
    )
    assert result["errors"] == {CONF_PROJECT_KEYS: "invalid_key"}

    key = "AB" * 32
    result = await hass.config_entries.options.async_configure(
        result["flow_id"], {CONF_PROJECT_KEYS: key}
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert owner.options[CONF_PROJECT_KEYS] == [key.lower()]
    # Saved options reload the entry, so the update entity sees the keys.
    await hass.async_block_till_done()
    assert await hass.config_entries.async_unload(owner.entry_id)
