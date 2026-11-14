"""The feeder's buttons: feed, cancel, reset the desiccant, chime.

There is no restart: the feeder lets only an owner restart it, and Home
Assistant joins as a member.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass

from homeassistant.components.button import ButtonEntity, ButtonEntityDescription
from homeassistant.const import EntityCategory
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .entity import WellfedEntity, serves
from .session import FeederSession

PARALLEL_UPDATES = 0


async def _feed(session: FeederSession) -> None:
    await session.feed(session.portions)


async def _cancel(session: FeederSession) -> None:
    if session.running_feed is not None:
        await session.cancel_feed(session.running_feed)


@dataclass(frozen=True, kw_only=True)
class WellfedButtonDescription(ButtonEntityDescription):
    press: Callable[[FeederSession], Awaitable[None]]
    # Whether the feeder and this client's role offer it.
    offered: Callable[[FeederSession], bool] = lambda _: True
    # Whether it can be pressed now.
    ready: Callable[[FeederSession], bool] = lambda _: True


BUTTONS = (
    WellfedButtonDescription(
        key="feed",
        translation_key="feed",
        press=_feed,
        offered=lambda session: serves(session, lambda caps: caps.feed),
    ),
    WellfedButtonDescription(
        key="cancel_feed",
        translation_key="cancel_feed",
        press=_cancel,
        offered=lambda session: serves(session, lambda caps: caps.feed),
        ready=lambda session: session.running_feed is not None,
    ),
    WellfedButtonDescription(
        key="reset_desiccant",
        translation_key="reset_desiccant",
        entity_category=EntityCategory.CONFIG,
        press=lambda session: session.reset_desiccant(),
        offered=lambda session: serves(session, lambda caps: caps.desiccant),
    ),
    WellfedButtonDescription(
        key="play_chime",
        translation_key="play_chime",
        entity_category=EntityCategory.CONFIG,
        press=lambda session: session.play(bridge.Tune.Chime),
    ),
)


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    async_add_entities(
        WellfedButton(session, description)
        for description in BUTTONS
        if description.offered(session)
    )


class WellfedButton(WellfedEntity, ButtonEntity):
    """Each press sends one request, under its own request id."""

    entity_description: WellfedButtonDescription

    def __init__(self, session: FeederSession, description: WellfedButtonDescription) -> None:
        super().__init__(session, description.key)
        self.entity_description = description

    @property
    def available(self) -> bool:
        return super().available and self.entity_description.ready(self.session)

    async def async_press(self) -> None:
        await self.entity_description.press(self.session)
