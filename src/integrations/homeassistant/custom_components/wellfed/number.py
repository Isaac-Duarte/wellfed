"""The feeder's numbers: the portions the Feed button asks for, the
volume, and the weight of one portion."""

from __future__ import annotations

from homeassistant.components.number import (
    NumberDeviceClass,
    NumberEntity,
    NumberMode,
    RestoreNumber,
)
from homeassistant.const import PERCENTAGE, EntityCategory, UnitOfMass
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

import wellfed_homeassistant as bridge

from . import WellfedConfigEntry
from .const import PORTIONS_MAX, PORTIONS_MIN
from .entity import WellfedEntity, serves

PARALLEL_UPDATES = 0

# The feeder's limits on a portion's weight, from the bridge, in grams.
PORTION_GRAMS_MIN = bridge.PORTION_MG_MIN / 1000
PORTION_GRAMS_MAX = bridge.PORTION_MG_MAX / 1000


async def async_setup_entry(
    hass: HomeAssistant,
    entry: WellfedConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    session = entry.runtime_data
    entities: list[WellfedEntity] = [Volume(session, "volume")]
    if serves(session, lambda caps: caps.feed):
        entities.append(Portions(session, "portions"))
    if serves(session, lambda caps: caps.calibration):
        entities.append(PortionWeight(session, "portion_weight"))
    async_add_entities(entities)


class Portions(WellfedEntity, RestoreNumber):
    """How many portions the Feed button asks for. Home Assistant's own
    choice, kept across restarts: the feeder stores no such number."""

    _attr_native_min_value = PORTIONS_MIN
    _attr_native_max_value = PORTIONS_MAX
    _attr_native_step = 1
    _attr_mode = NumberMode.BOX

    @property
    def available(self) -> bool:
        return True

    async def async_added_to_hass(self) -> None:
        await super().async_added_to_hass()
        restored = await self.async_get_last_number_data()
        if restored is not None and restored.native_value is not None:
            self.session.portions = int(restored.native_value)

    @property
    def native_value(self) -> float:
        return self.session.portions

    async def async_set_native_value(self, value: float) -> None:
        self.session.portions = int(value)
        self.async_write_ha_state()


class Volume(WellfedEntity, NumberEntity):
    """The speaker's volume, as the feeder's settings hold it."""

    _attr_entity_category = EntityCategory.CONFIG
    _attr_native_min_value = 0
    _attr_native_max_value = 100
    _attr_native_step = 1
    _attr_native_unit_of_measurement = PERCENTAGE

    @property
    def native_value(self) -> float | None:
        state = self.session.state
        settings = state.settings if state is not None else None
        return settings.volume if settings is not None else None

    async def async_set_native_value(self, value: float) -> None:
        await self.session.put_settings(volume=int(value))


class PortionWeight(WellfedEntity, NumberEntity):
    """The weight of one portion, as it was measured."""

    _attr_entity_category = EntityCategory.CONFIG
    _attr_device_class = NumberDeviceClass.WEIGHT
    _attr_native_min_value = PORTION_GRAMS_MIN
    _attr_native_max_value = PORTION_GRAMS_MAX
    _attr_native_step = 0.1
    _attr_native_unit_of_measurement = UnitOfMass.GRAMS
    _attr_mode = NumberMode.BOX

    @property
    def native_value(self) -> float | None:
        state = self.session.state
        if state is None or state.portion_mg is None:
            return None
        return state.portion_mg / 1000

    async def async_set_native_value(self, value: float) -> None:
        await self.session.set_portion_weight(round(value * 1000))
