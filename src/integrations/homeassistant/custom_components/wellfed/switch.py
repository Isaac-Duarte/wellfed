"""The child lock, sound, lights, and a switch for each feeding plan.

Every state is the feeder's, never assumed after a command: a switch moves
when the feeder's state does. Plan switches follow the plan table: a plan
added on the feeder adds its switch, and a deleted one removes it.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any

from homeassistant.components.switch import SwitchEntity, SwitchEntityDescription
from homeassistant.const import EntityCategory
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import entity_registry as er
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .const import DOMAIN
from .entity import WellfedEntity, serves
from .session import FeederSession

PARALLEL_UPDATES = 0


def _settings(session: FeederSession) -> bridge.Settings | None:
    return session.state.settings if session.state is not None else None


@dataclass(frozen=True, kw_only=True)
class WellfedSwitchDescription(SwitchEntityDescription):
    value: Callable[[FeederSession], bool | None]
    turn: Callable[[FeederSession, bool], Awaitable[None]]


SWITCHES = (
    WellfedSwitchDescription(
        key="child_lock",
        translation_key="child_lock",
        entity_category=EntityCategory.CONFIG,
        value=lambda session: session.status.locked if session.status is not None else None,
        turn=lambda session, on: session.set_lock(on),
    ),
    WellfedSwitchDescription(
        key="sound",
        translation_key="sound",
        entity_category=EntityCategory.CONFIG,
        value=lambda session: (settings := _settings(session)) and settings.sound,
        turn=lambda session, on: session.put_settings(sound=on),
    ),
    WellfedSwitchDescription(
        key="lights",
        translation_key="lights",
        entity_category=EntityCategory.CONFIG,
        value=lambda session: (settings := _settings(session)) and settings.lights,
        turn=lambda session, on: session.put_settings(lights=on),
    ),
)


def _plans(session: FeederSession) -> dict[int, bridge.Plan]:
    state = session.state
    if state is None or state.plans is None:
        return {}
    return {plan.slot: plan for plan in state.plans}


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    async_add_entities(WellfedSwitch(session, description) for description in SWITCHES)
    if not serves(session, lambda caps: caps.plans):
        return

    shown: set[int] = set()
    registry = er.async_get(hass)

    @callback
    def follow_plans() -> None:
        """Add a switch for each new plan; remove the switch of each gone."""
        slots = set(_plans(session))
        added = slots - shown
        if added:
            async_add_entities(PlanSwitch(session, slot) for slot in sorted(added))
        for slot in shown - slots:
            unique_id = PlanSwitch.unique_id_of(session, slot)
            entity_id = registry.async_get_entity_id("switch", DOMAIN, unique_id)
            if entity_id is not None:
                registry.async_remove(entity_id)
        shown.clear()
        shown.update(slots)

    follow_plans()
    entry.async_on_unload(session.add_listener(follow_plans))


class WellfedSwitch(WellfedEntity, SwitchEntity):
    _writes = True
    entity_description: WellfedSwitchDescription

    def __init__(self, session: FeederSession, description: WellfedSwitchDescription) -> None:
        super().__init__(session, description.key)
        self.entity_description = description

    @property
    def is_on(self) -> bool | None:
        return self.entity_description.value(self.session)

    async def async_turn_on(self, **kwargs: Any) -> None:
        await self.entity_description.turn(self.session, True)

    async def async_turn_off(self, **kwargs: Any) -> None:
        await self.entity_description.turn(self.session, False)


class PlanSwitch(WellfedEntity, SwitchEntity):
    """One feeding plan, on or off, named after its time: "Meal 07:30"."""

    _writes = True
    _attr_entity_category = EntityCategory.CONFIG

    def __init__(self, session: FeederSession, slot: int) -> None:
        super().__init__(session, f"plan_{slot}")
        self.slot = slot
        self._attr_translation_key = "plan"
        self._attr_translation_placeholders = self._placeholders()

    @staticmethod
    def unique_id_of(session: FeederSession, slot: int) -> str:
        return f"{session.identity.hex()}-plan_{slot}"

    @property
    def _plan(self) -> bridge.Plan | None:
        return _plans(self.session).get(self.slot)

    def _placeholders(self) -> dict[str, str]:
        plan = self._plan
        if plan is None:
            return {"time": "--:--", "portions": "?"}
        hours, minutes = divmod(plan.minute, 60)
        return {"time": f"{hours:02}:{minutes:02}", "portions": str(plan.portions)}

    async def async_added_to_hass(self) -> None:
        await super().async_added_to_hass()
        self.async_on_remove(self.session.add_listener(self._rename))

    @callback
    def _rename(self) -> None:
        """A plan moved to another time is named after its new one."""
        self._attr_translation_placeholders = self._placeholders()
        self.async_write_ha_state()

    @property
    def available(self) -> bool:
        return super().available and self._plan is not None

    @property
    def is_on(self) -> bool | None:
        plan = self._plan
        return plan.enabled if plan is not None else None

    async def async_turn_on(self, **kwargs: Any) -> None:
        await self.session.set_plan_enabled(self.slot, True)

    async def async_turn_off(self, **kwargs: Any) -> None:
        await self.session.set_plan_enabled(self.slot, False)
