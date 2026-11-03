"""Firmware updates, in owner mode: the installed build, the newest
verified release, and an install with its progress.

Releases come from the project's GitHub releases, checked a few times a
day when the integration's options name the project's keys, or from a file
the owner adds with the `wellfed.add_release` action. Each is verified by
the bridge with the feeder's own offer check; an install is the bridge's
too, sent once and never resent behind the owner's back.
"""

from __future__ import annotations

from datetime import timedelta
import logging
from typing import Any

from homeassistant.components.update import UpdateEntity, UpdateEntityFeature
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry, project_keys, release_folder
from .const import DOMAIN
from .entity import WellfedEntity, owner_mode, serves
from .session import FeederSession

_LOGGER = logging.getLogger(__name__)

PARALLEL_UPDATES = 0
SCAN_INTERVAL = timedelta(hours=6)

# A build id is 64 hex digits; this many name it to a person.
BUILD_SHOWN = 12


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    if owner_mode(session) and serves(session, lambda caps: caps.update):
        firmware = Firmware(session, "firmware")
        firmware.folder = release_folder(hass, entry)
        firmware.keys = project_keys(entry)
        async_add_entities([firmware], update_before_add=True)


class Firmware(WellfedEntity, UpdateEntity):
    """The feeder's firmware, with the newest release this Home Assistant
    holds and trusts."""

    _attr_should_poll = True
    _attr_supported_features = UpdateEntityFeature.INSTALL | UpdateEntityFeature.PROGRESS

    def __init__(self, session: FeederSession, key: str) -> None:
        super().__init__(session, key)
        self.folder = ""
        self.keys: list[bytes] = []
        self.offer: bridge.UpdateOffer | None = None
        self.installing = False

    async def async_update(self) -> None:
        """Look for a newer release: on GitHub when the project's keys are
        set, and among the files the owner added."""
        try:
            self.offer = await self.session.newest_release(
                self.folder, self.keys, fetch=bool(self.keys)
            )
        except (bridge.WellfedError, HomeAssistantError) as error:
            _LOGGER.warning("Checking for firmware releases failed: %s", error)

    @property
    def installed_version(self) -> str | None:
        status = self.session.status
        return status.build[:BUILD_SHOWN] if status is not None else None

    @property
    def latest_version(self) -> str | None:
        if self.offer is None:
            return self.installed_version
        return self.offer.build[:BUILD_SHOWN]

    @property
    def release_summary(self) -> str | None:
        if self.offer is None:
            return None
        return f"Release {self.offer.version}" + (f" ({self.offer.tag})" if self.offer.tag else "")

    @property
    def in_progress(self) -> bool:
        return self.installing

    @property
    def update_percentage(self) -> int | None:
        progress = self.session.install_progress
        if not self.installing or progress is None or progress.image_len == 0:
            return None
        return min(100, progress.written * 100 // progress.image_len)

    async def async_install(self, version: str | None, backup: bool, **kwargs: Any) -> None:
        offer = self.offer
        if offer is None:
            raise HomeAssistantError(translation_domain=DOMAIN, translation_key="no_release")
        self.installing = True
        self.async_write_ha_state()
        try:
            installed = await self.session.install(self.folder, self.keys, offer.id)
        finally:
            self.installing = False
            self.async_write_ha_state()
        if installed.result not in (bridge.Installed.Activated, bridge.Installed.AlreadyRunning):
            raise HomeAssistantError(
                translation_domain=DOMAIN,
                translation_key=f"install_{installed.result.key}",
            )
