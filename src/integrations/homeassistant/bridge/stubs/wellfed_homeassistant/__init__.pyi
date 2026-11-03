from typing import Final, SupportsFloat, SupportsIndex, final
from typing_extensions import disjoint_base

__all__ = ["By", "Event", "EventKind", "Feed", "FeedFault", "FeedOutcome", "FeedReason", "Feeder", "FoodLevel", "Readiness", "RefusalKind", "Role", "Status", "Update", "UpdateKind", "WellfedError", "WriteResult", "Written", "generate_secret", "invite_feeder", "join", "public_key"]

@final
class By:
    """
    Who asked for a feed or a change.
    """
    Client: Final[By]
    """
    A client; the event names its fingerprint.
    """
    Feeder: Final[By]
    Panel: Final[By]
    Plan: Final[By]
    """
    A feeding plan; the event names its slot.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Event:
    """
    One event from the feeder's history.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def at_ms(self, /) -> int | None:
        """
        When it happened, by the feeder's clock in Unix milliseconds;
        `None` while that was invalid.
        """
    @property
    def by(self, /) -> By | None:
        """
        Who made the change or asked for the feed, when the event says.
        """
    @property
    def client(self, /) -> str | None:
        """
        The client `by` names, or the client enrolled or revoked.
        """
    @property
    def fed(self, /) -> Feed | None:
        """
        The feed, for [`EventKind::Fed`].
        """
    @property
    def kind(self, /) -> EventKind: ...
    @property
    def locked(self, /) -> bool | None:
        """
        The panel's lock after a [`EventKind::Lock`].
        """
    @property
    def plan(self, /) -> int | None:
        """
        The plan `by` names, or the plan missed or due soon.
        """
    @property
    def seq(self, /) -> int:
        """
        The feeder's sequence number, rising across its reboots.
        """

@final
class EventKind:
    """
    What an event records.
    """
    Boot: Final[EventKind]
    ClockSync: Final[EventKind]
    DispenseRefused: Final[EventKind]
    Enrolled: Final[EventKind]
    Fed: Final[EventKind]
    FeedingBlocked: Final[EventKind]
    Food: Final[EventKind]
    Lock: Final[EventKind]
    Lost: Final[EventKind]
    MealSoon: Final[EventKind]
    Missed: Final[EventKind]
    Plans: Final[EventKind]
    Revoked: Final[EventKind]
    Settings: Final[EventKind]
    Supply: Final[EventKind]
    Wifi: Final[EventKind]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Feed:
    """
    A feed that ended, from its `Fed` event.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def at_ms(self, /) -> int | None:
        """
        When it ended, by the feeder's clock in Unix milliseconds; `None`
        while that was invalid.
        """
    @property
    def by(self, /) -> By: ...
    @property
    def client(self, /) -> str | None:
        """
        The fingerprint of the client that asked, for [`By::Client`].
        """
    @property
    def completed(self, /) -> int:
        """
        The portions the feedback sensor confirmed.
        """
    @property
    def feed(self, /) -> int:
        """
        The feeder's number for the feed.
        """
    @property
    def hopper(self, /) -> FoodLevel:
        """
        What the food beam read during the feed.
        """
    @property
    def outcome(self, /) -> FeedOutcome: ...
    @property
    def plan(self, /) -> int | None:
        """
        The plan's slot, for [`By::Plan`].
        """
    @property
    def reason(self, /) -> FeedReason: ...
    @property
    def requested(self, /) -> int: ...

@final
class FeedFault:
    """
    A feed that ended on a fault; its `Fed` event has the rest.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def feed(self, /) -> int: ...
    @property
    def outcome(self, /) -> FeedOutcome: ...
    @property
    def reason(self, /) -> FeedReason: ...

@final
class FeedOutcome:
    """
    How a feed ended.
    """
    Cancelled: Final[FeedOutcome]
    Done: Final[FeedOutcome]
    Expired: Final[FeedOutcome]
    Failed: Final[FeedOutcome]
    Uncertain: Final[FeedOutcome]
    """
    It may have run: the feeder cannot tell how much.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class FeedReason:
    """
    Why a feed ended as it did.
    """
    Cancelled: Final[FeedReason]
    Clock: Final[FeedReason]
    Done: Final[FeedReason]
    FeedbackStream: Final[FeedReason]
    Interrupted: Final[FeedReason]
    Jammed: Final[FeedReason]
    MarkStuck: Final[FeedReason]
    NoFeedback: Final[FeedReason]
    ObservationGap: Final[FeedReason]
    OutletBlocked: Final[FeedReason]
    Output: Final[FeedReason]
    Overcurrent: Final[FeedReason]
    Power: Final[FeedReason]
    Sensor: Final[FeedReason]
    Timeout: Final[FeedReason]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Feeder:
    """
    One feeder's session, on a thread of its own until this is dropped.
    """
    def __new__(cls, /, secret: bytes, identity: bytes, address: str | None = None) -> Feeder: ...
    def feed(self, /, portions: SupportsIndex) -> Written:
        """
        Start a feed; how it ended. Never retry an uncertain one.
        """
    def next_update(self, /, timeout: SupportsFloat | SupportsIndex) -> Update | None:
        """
        The next update within `timeout` seconds, or `None`.
        """
    def refresh(self, /) -> None:
        """
        Read the status now.
        """
    def set_lock(self, /, locked: bool) -> Written:
        """
        Lock or unlock the panel; how it ended.
        """

@final
class FoodLevel:
    """
    The hopper as the food beam last read it.
    """
    Low: Final[FoodLevel]
    """
    About five portions remain.
    """
    Present: Final[FoodLevel]
    Unread: Final[FoodLevel]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Readiness:
    """
    Whether a feed can start now, and if not why: the feeder's
    `Readiness` with its blocked reason flattened in, one state for an
    enum sensor.
    """
    Battery: Final[Readiness]
    Fault: Final[Readiness]
    Feeding: Final[Readiness]
    Joining: Final[Readiness]
    Journal: Final[Readiness]
    Ready: Final[Readiness]
    Resolving: Final[Readiness]
    Restarting: Final[Readiness]
    Setup: Final[Readiness]
    Starting: Final[Readiness]
    Unserved: Final[Readiness]
    Updating: Final[Readiness]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class RefusalKind:
    """
    Why the feeder refused a write.
    """
    Busy: Final[RefusalKind]
    Fault: Final[RefusalKind]
    Forbidden: Final[RefusalKind]
    Full: Final[RefusalKind]
    Invalid: Final[RefusalKind]
    Muted: Final[RefusalKind]
    NoClock: Final[RefusalKind]
    NotFound: Final[RefusalKind]
    NotWritable: Final[RefusalKind]
    Portions: Final[RefusalKind]
    Stale: Final[RefusalKind]
    Time: Final[RefusalKind]
    Unavailable: Final[RefusalKind]
    Unverified: Final[RefusalKind]
    Update: Final[RefusalKind]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Role:
    """
    The role the feeder enrolled this client with.
    """
    Member: Final[Role]
    Owner: Final[Role]
    Server: Final[Role]
    UpdateSigner: Final[Role]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Status:
    """
    What the feeder reports about itself.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def adapter(self, /) -> bool | None:
        """
        On the adapter; `None` with no fresh sample or no supply report.
        """
    @property
    def bank(self, /) -> int:
        """
        The flash bank it booted from.
        """
    @property
    def battery_percent(self, /) -> int | None:
        """
        The pack's charge, 0-100 in steps of 10, while it reads one.
        """
    @property
    def below_floor(self, /) -> bool | None:
        """
        Whether the pack is below the floor a feed needs, once judged.
        """
    @property
    def boot_count(self, /) -> int: ...
    @property
    def build(self, /) -> str: ...
    @property
    def clock_ms(self, /) -> int | None:
        """
        The feeder's clock, Unix milliseconds; `None` while it is invalid.
        """
    @property
    def desiccant_days(self, /) -> int | None:
        """
        Days left until the desiccant is due, below zero once overdue;
        `None` before a first replacement or while the clock is invalid.
        """
    @property
    def desiccant_due(self, /) -> bool: ...
    @property
    def feed_fault(self, /) -> FeedFault | None:
        """
        The last feed this boot that ended on a fault, until a later feed
        completes.
        """
    @property
    def food(self, /) -> FoodLevel: ...
    @property
    def last_event(self, /) -> int | None:
        """
        The newest event's sequence number.
        """
    @property
    def locked(self, /) -> bool | None:
        """
        The panel's child lock; `None` before it is known.
        """
    @property
    def readiness(self, /) -> Readiness | None:
        """
        `None` from a feeder that does not report it.
        """
    @property
    def rssi(self, /) -> int:
        """
        Wi-Fi signal, dBm.
        """
    @property
    def sessions(self, /) -> int:
        """
        Open device API sessions, this one included.
        """
    @property
    def uptime_ms(self, /) -> int: ...

@final
class Update:
    """
    What the session tells Home Assistant, in order.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def build(self, /) -> str | None:
        """
        The build that answered, for [`UpdateKind::Connected`].
        """
    @property
    def event(self, /) -> Event | None: ...
    @property
    def gap_from(self, /) -> int | None:
        """
        The first and last event lost, for a [`UpdateKind::Gap`] the feeder
        named; both `None` for a history that started over.
        """
    @property
    def gap_to(self, /) -> int | None: ...
    @property
    def kind(self, /) -> UpdateKind: ...
    @property
    def reason(self, /) -> str | None:
        """
        Why the session ended, for [`UpdateKind::Disconnected`].
        """
    @property
    def role(self, /) -> Role | None:
        """
        The role this client holds, for [`UpdateKind::Connected`].
        """
    @property
    def status(self, /) -> Status | None: ...

@final
class UpdateKind:
    """
    What a session update carries.
    """
    Connected: Final[UpdateKind]
    Disconnected: Final[UpdateKind]
    Event: Final[UpdateKind]
    Gap: Final[UpdateKind]
    """
    Events this session never received: what happened meanwhile is
    not known.
    """
    Status: Final[UpdateKind]
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@disjoint_base
class WellfedError(Exception):
    """
    A call into the Wellfed bridge failed.

    A class rather than `create_exception!`, so the stub describes it.
    """
    def __new__(cls, /, *_args) -> WellfedError: ...

@final
class WriteResult:
    """
    How a write ended.
    """
    Done: Final[WriteResult]
    """
    The write took effect.
    """
    Feeding: Final[WriteResult]
    """
    A feed is reserved; its `Fed` event carries the result.
    """
    NotSent: Final[WriteResult]
    """
    It never went out: no session was open.
    """
    Refused: Final[WriteResult]
    Uncertain: Final[WriteResult]
    """
    It went out and no answer came: it may have run. Never resend it.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __hash__(self, /) -> int: ...
    def __int__(self, /) -> int: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __repr__(self, /) -> str: ...
    @property
    def key(self, /) -> str:
        """
        The snake_case name: an entity's state and its translation key.
        """

@final
class Written:
    """
    How a write ended.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def feed(self, /) -> int | None:
        """
        The feeder's number for a feed it reserved.
        """
    @property
    def reason(self, /) -> str | None:
        """
        Why it never went out, for [`WriteResult::NotSent`].
        """
    @property
    def refusal(self, /) -> RefusalKind | None:
        """
        Why it refused, for [`WriteResult::Refused`].
        """
    @property
    def result(self, /) -> WriteResult: ...

def generate_secret() -> bytes:
    """
    A new client key's secret, for the config entry.
    """

def invite_feeder(text: str) -> bytes:
    """
    The identity key of the feeder `text` invites to.
    """

def join(secret: bytes, invite: str, label: str, address: str | None = None) -> Role:
    """
    Redeem `invite` as the key `secret`, listed as `label`; the role the
    feeder enrolled it with. Runs once: an invite is single-use.
    """

def public_key(secret: bytes) -> bytes:
    """
    The public key of `secret`, as the feeder lists this client.
    """
