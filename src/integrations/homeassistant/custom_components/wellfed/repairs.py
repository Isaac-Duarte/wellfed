"""The repair that releases a lost feed.

Directly, a feed whose answer was lost holds new feeds back until its `Fed`
event settles it, as the app holds one: it may have run, and the feeder's
history names this Home Assistant, not the request. Once the history holds
no record of it, its repair lets the owner release it, so feeds go again
though it may have run. Nothing else releases it.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from homeassistant.components.repairs import RepairsFlow, RepairsFlowResult
from homeassistant.core import HomeAssistant
from homeassistant.helpers import issue_registry as ir
import probatio

from .const import DOMAIN

if TYPE_CHECKING:
    from .session import FeederSession


def _session(hass: HomeAssistant, identity: str) -> FeederSession | None:
    """The loaded entry's session for the feeder `identity`, in hex."""
    for entry in hass.config_entries.async_loaded_entries(DOMAIN):
        session: FeederSession = entry.runtime_data
        if session.identity.hex() == identity:
            return session
    return None


class ReleaseFlow(RepairsFlow):
    """Release the lost feeds of one feeder that hold new feeds back."""

    def __init__(self, identity: str) -> None:
        self._identity = identity

    async def async_step_init(self, user_input: dict[str, str] | None = None) -> RepairsFlowResult:
        return await self.async_step_confirm()

    async def async_step_confirm(
        self, user_input: dict[str, str] | None = None
    ) -> RepairsFlowResult:
        if user_input is None:
            issue = ir.async_get(self.hass).async_get_issue(DOMAIN, self.issue_id)
            return self.async_show_form(
                step_id="confirm",
                data_schema=probatio.Schema({}),
                description_placeholders=issue.translation_placeholders if issue else None,
            )
        session = _session(self.hass, self._identity)
        if session is None:
            return self.async_abort(reason="not_loaded")
        session.release_unknown()
        return self.async_create_entry(data={})


async def async_create_fix_flow(
    hass: HomeAssistant, issue_id: str, data: dict[str, str | int | float | None] | None
) -> RepairsFlow:
    """The fix flow for a fixable issue: only the lost-feed repair is one."""
    identity = data.get("identity") if data is not None else None
    return ReleaseFlow(identity if isinstance(identity, str) else "")
