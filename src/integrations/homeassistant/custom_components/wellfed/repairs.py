"""The repairs that release a hold on feeds.

A feed whose answer was lost holds new feeds back on either route until it
settles, as the app holds one: it may have run, and the feeder's history
names this Home Assistant, not the request. Once no record of it turns up,
its repair lets the owner release it, so feeds go again though it may have
run. An entry whose store did not read holds every feed, since any lost
feed it kept went with it, until the owner says the bowls are checked.
Nothing else releases either.
"""

from __future__ import annotations

from homeassistant.components.repairs import RepairsFlow, RepairsFlowResult
from homeassistant.core import HomeAssistant
from homeassistant.helpers import issue_registry as ir
import probatio

from .const import DOMAIN
from .session import STORE_UNREAD_ISSUE, FeederSession


def _session(hass: HomeAssistant, identity: str) -> FeederSession | None:
    """The loaded entry's session for the feeder `identity`, in hex."""
    for entry in hass.config_entries.async_loaded_entries(DOMAIN):
        session: FeederSession = entry.runtime_data
        if session.identity.hex() == identity:
            return session
    return None


class ReleaseFlow(RepairsFlow):
    """Release what holds one feeder's feeds back: its lost feeds, or the
    unread store's hold, as `repair` names."""

    def __init__(self, identity: str, repair: str) -> None:
        self._identity = identity
        self._repair = repair

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
        if self._repair == STORE_UNREAD_ISSUE:
            session.check_bowls()
        else:
            session.release_unknown()
        return self.async_create_entry(data={})


async def async_create_fix_flow(
    hass: HomeAssistant, issue_id: str, data: dict[str, str | int | float | None] | None
) -> RepairsFlow:
    """The fix flow for a fixable issue: each releases a hold."""
    identity = data.get("identity") if data is not None else None
    repair = data.get("repair") if data is not None else None
    return ReleaseFlow(
        identity if isinstance(identity, str) else "",
        repair if isinstance(repair, str) else "",
    )
