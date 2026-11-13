from _typeshed import SupportsGetItem
from typing import Final, SupportsFloat, SupportsIndex, final
from typing_extensions import disjoint_base

__all__ = ["PORTIONS_MAX", "PORTIONS_MIN", "PORTION_MG_MAX", "PORTION_MG_MIN", "Alert", "AlertKind", "AlertLevel", "Alerts", "By", "Capabilities", "Event", "EventKind", "Feed", "FeedFault", "FeedOutcome", "FeedReason", "Feeder", "FoodLevel", "HeldKey", "JoinFailed", "JoinProblem", "Joined", "NextMeal", "NotSent", "Pending", "Plan", "PortionsToday", "Readiness", "Redeemed", "RefusalKind", "Role", "Settings", "Skew", "State", "Status", "Tune", "Update", "UpdateKind", "WellfedError", "WriteResult", "Written", "invite_feeder", "public_key", "redeem"]

PORTIONS_MAX: Final[int]
"""
The most portions one feed asks for.
"""

PORTIONS_MIN: Final[int]
"""
The fewest portions one feed asks for.
"""

PORTION_MG_MAX: Final[int]
"""
The heaviest portion weight the feeder stores, in milligrams.
"""

PORTION_MG_MIN: Final[int]
"""
The lightest portion weight the feeder stores, in milligrams.
"""

@final
class Alert:
    """
    One alert.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def at_ms(self, /) -> int | None:
        """
        The feeder's time for it, Unix milliseconds; `None` without a valid
        clock then.
        """
    @property
    def key(self, /) -> str:
        """
        The alert's identity on its feeder, the same in the event that opens
        it and the one that clears it.
        """
    @property
    def kind(self, /) -> AlertKind: ...
    @property
    def level(self, /) -> AlertLevel: ...

@final
class AlertKind:
    """
    What an alert is about, as the app's tracker names it. `Unreachable`
    is a relay's judgement that it can't reach the feeder, which a
    direct session never makes: its Connected sensor says so.
    """
    BatteryBelowFloor: Final[AlertKind]
    CannotFeed: Final[AlertKind]
    ClientJoined: Final[AlertKind]
    ClientRevoked: Final[AlertKind]
    DesiccantDue: Final[AlertKind]
    FoodLikelyOut: Final[AlertKind]
    FoodLow: Final[AlertKind]
    MealFailed: Final[AlertKind]
    MealMissed: Final[AlertKind]
    MealNoRecord: Final[AlertKind]
    MealUncertain: Final[AlertKind]
    NoClock: Final[AlertKind]
    PlansStuck: Final[AlertKind]
    PowerWasOut: Final[AlertKind]
    RepeatedRestarts: Final[AlertKind]
    RolledBack: Final[AlertKind]
    Unreachable: Final[AlertKind]
    UpdateFailed: Final[AlertKind]
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
class AlertLevel:
    """
    How soon an alert wants the owner.
    """
    Security: Final[AlertLevel]
    Soon: Final[AlertLevel]
    Urgent: Final[AlertLevel]
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
class Alerts:
    """
    The feeder's alerts after a read: what is open, and what opened and
    cleared since the read before.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def cleared(self, /) -> list[Alert]: ...
    @property
    def open(self, /) -> list[Alert]:
        """
        Open now, oldest first.
        """
    @property
    def opened(self, /) -> list[Alert]: ...
    @property
    def unread(self, /) -> int:
        """
        Alerts the feeder holds that this integration is too old to read.
        """

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
    NewerBuild: Final[By]
    """
    A value a newer build added, which this integration cannot name.
    """
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
class Capabilities:
    """
    What the feeder serves this session, as Home Assistant chooses its
    entities by: hints from the device's kind. A write is gated by its op's
    own kind (`writes::served`), never by these.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def calibration(self, /) -> bool: ...
    @property
    def desiccant(self, /) -> bool: ...
    @property
    def feed(self, /) -> bool: ...
    @property
    def plans(self, /) -> bool: ...
    @property
    def time(self, /) -> bool: ...
    @property
    def update(self, /) -> bool: ...

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
    Invited: Final[EventKind]
    Lock: Final[EventKind]
    Lost: Final[EventKind]
    MealSoon: Final[EventKind]
    Missed: Final[EventKind]
    Plans: Final[EventKind]
    Restarting: Final[EventKind]
    Revoked: Final[EventKind]
    Settings: Final[EventKind]
    Skipped: Final[EventKind]
    Supply: Final[EventKind]
    Unskipped: Final[EventKind]
    UpdateAtBoot: Final[EventKind]
    UpdateConfirmed: Final[EventKind]
    UpdateStop: Final[EventKind]
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
    A feed that ended, from its `Fed` event, as the feeding history reads
    it: an uncertain feed is a range of portions, never one count.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def at_most(self, /) -> int:
        """
        The most that may have dropped: `completed` unless the feed is
        uncertain, when it is what was asked for.
        """
    @property
    def at_ms(self, /) -> int | None:
        """
        When it ended, by the feeder's clock in Unix milliseconds; `None`
        while that was invalid.
        """
    @property
    def by(self, /) -> By | None:
        """
        Who asked; `None` for a feed a reset interrupted, whose requester
        the feeder's journal does not keep.
        """
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
    NewerBuild: Final[FeedOutcome]
    """
    A value a newer feeder build added, which this integration
    cannot name: update it.
    """
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
    NewerBuild: Final[FeedReason]
    """
    A value a newer feeder build added, which this integration
    cannot name: update it.
    """
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
    def __new__(cls, /, secret: bytes, identity: bytes, address: str | None = None, pending: SupportsGetItem[int, Pending] = ..., answered: SupportsGetItem[int, SupportsIndex] = ..., unread: bool = False) -> Feeder:
        """
        `pending` and `answered` are what the last run left: the feeds still
        unsettled, and the feed numbers the last `Answered` update listed.
        `unread` says the integration's store did not read, and the owner
        has not yet checked the bowls: every feed waits until
        `bowls_checked`.
        """
    def bowls_checked(self, /) -> None:
        """
        The owner checked the bowls after the integration's store did not
        read, through its repair: feeds go again. A `BowlsChecked` update
        confirms it.
        """
    def cancel_feed(self, /, feed: SupportsIndex) -> Written:
        """
        Stop the feed the feeder numbered `feed`, if it still runs.
        """
    def feed(self, /, portions: SupportsIndex) -> Written:
        """
        Start a feed; how it ended. Never retry an uncertain one. It goes
        out only once `saved` confirms the `Sending` update's record is in
        the store.
        """
    def next_update(self, /, timeout: SupportsFloat | SupportsIndex) -> Update | None:
        """
        The next update within `timeout` seconds, or `None`.
        """
    def play(self, /, tune: Tune) -> Written: ...
    def put_settings(self, /, *, sound: bool | None = None, lights: bool | None = None, volume: SupportsIndex | None = None, desiccant_days: SupportsIndex | None = None) -> Written:
        """
        Change the settings named, on the settings last read; the rest
        stay as they are. One settings change at a time: another while it
        waits is not sent.
        """
    def refresh(self, /) -> None:
        """
        Read the status now.
        """
    def release(self, /, id: str) -> None:
        """
        The owner releases the lost feed `id` names, through its repair: new
        feeds go again though it may have run. A `Released` update confirms
        it.
        """
    def reset_desiccant(self, /) -> Written:
        """
        Record a fresh desiccant pack today.
        """
    def saved(self, /, id: str) -> None:
        """
        The integration's store saved the record of the feed `id` names, as
        a `Sending` update asked: the feed goes out. Called from the event
        loop while `feed` blocks an executor thread.
        """
    def set_lock(self, /, locked: bool) -> Written:
        """
        Lock or unlock the panel; how it ended.
        """
    def set_plan_enabled(self, /, slot: SupportsIndex, enabled: bool) -> Written:
        """
        Turn the plan in `slot` on or off, on the plans last read.
        """
    def set_portion_weight(self, /, mg: SupportsIndex) -> Written:
        """
        Store the weight of one portion in milligrams, 1,000-50,000.
        """
    def unsaved(self, /, id: str) -> None:
        """
        The integration's store could not save the record of the feed `id`
        names: it never goes out, and `feed` answers it not sent.
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
    NewerBuild: Final[FoodLevel]
    """
    A value a newer feeder build added, which this integration
    cannot name: update it.
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
class HeldKey:
    """
    A key an unchecked join may have enrolled on `feeder`: the setup flow
    holds it and hands it to the next `redeem`, which checks it before it
    spends another invite for that feeder.
    """
    def __new__(cls, /, secret: SupportsGetItem[int, SupportsIndex], feeder: SupportsGetItem[int, SupportsIndex]) -> HeldKey: ...
    @property
    def feeder(self, /) -> bytes: ...
    def public_key(self, /) -> bytes:
        """
        The public key, as the feeder would list it.
        """
    @property
    def secret(self, /) -> bytes: ...

@final
class JoinFailed(WellfedError):
    """
    Redeeming an invite did not enroll Home Assistant's key; `problem` says
    how, as the setup flow words it, and `held` is the key the next `redeem`
    must be handed, if any.
    """
    def __new__(cls, /, problem: JoinProblem, held: HeldKey | None, *_args) -> JoinFailed: ...
    @property
    def held(self, /) -> HeldKey | None: ...
    @property
    def problem(self, /) -> JoinProblem: ...

@final
class JoinProblem:
    """
    Why redeeming an invite did not enroll Home Assistant's key, as the
    setup flow words it.
    """
    NotAccepted: Final[JoinProblem]
    """
    The feeder did not take the invite (unknown, expired or spent):
    the key is not enrolled.
    """
    Uncertain: Final[JoinProblem]
    """
    The join went, its answer was lost, and no session could check
    it: the key may be enrolled, so the flow keeps it and asks again
    before it spends another invite.
    """
    Unreachable: Final[JoinProblem]
    """
    The feeder was not found, did not answer, or failed before the
    join went: nothing was enrolled.
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
class Joined:
    """
    How a member join ended.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def removed(self, /) -> bool:
        """
        For any role but member: whether the key was removed from the
        feeder again. When it was not, a key with that role stays listed
        (as the label it joined with) until someone revokes it.
        """
    @property
    def role(self, /) -> Role:
        """
        The role the invite enrolled the key with.
        """

@final
class NextMeal:
    """
    The plans' next meal, by the feeder's clock.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def at_ms(self, /) -> int:
        """
        Unix milliseconds.
        """
    @property
    def minute(self, /) -> int:
        """
        The minute of the feeder's local day it falls on.
        """
    @property
    def portions(self, /) -> int: ...
    @property
    def slot(self, /) -> int: ...

@final
class NotSent:
    """
    Why a write never went out. Each is safe to try again: the feeder
    saw nothing of it.
    """
    Busy: Final[NotSent]
    """
    A change to the same store is still waiting on its answer.
    """
    Disconnected: Final[NotSent]
    """
    No session is open now.
    """
    Forbidden: Final[NotSent]
    """
    This client's role may not do that.
    """
    Held: Final[NotSent]
    """
    Another feed of this client's is still being answered or
    settled, a lost one is not yet released, or the integration's
    store did not read. A new feed waits until that settles or the
    owner releases it, so the next `Fed` this client is named in can
    settle only that one, and a feed refused unseen is never taken
    for one that ran.
    """
    Invalid: Final[NotSent]
    """
    A value outside what the feeder accepts.
    """
    Legacy: Final[NotSent]
    """
    The feeder runs a build from before API revisions, which takes
    nothing but an update: update the feeder.
    """
    NotRead: Final[NotSent]
    """
    What the change is based on has not been read yet.
    """
    NotSaved: Final[NotSent]
    """
    The integration's store did not say it saved the feed's record,
    so the feed could outlive its hold across a restart: it never
    went.
    """
    Stopped: Final[NotSent]
    """
    The session has stopped for good: its handle is gone, or the
    feeder revoked this client.
    """
    Unsupported: Final[NotSent]
    """
    The feeder's firmware does not serve it.
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
class Pending:
    """
    A feed whose end is not known, until it is settled: what the
    integration keeps across a restart, and hands back to the next session
    so it is settled then. A feed is kept from just before it goes out
    ([`UpdateKind::Sending`]), and goes only once the store says it saved
    it, so a restart before its answer finds it lost rather than forgotten.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __new__(cls, /, id: str, portions: SupportsIndex, sent_ms: SupportsIndex, after: SupportsIndex | None, reported: bool, released_ms: SupportsIndex | None) -> Pending:
        """
        A pending feed as the integration kept it, to hand back to a new
        session.
        """
    @property
    def after(self, /) -> int | None:
        """
        The newest event known when it went out, which its `Fed` comes
        after.
        """
    @property
    def id(self, /) -> str:
        """
        Names it, for the store and the owner's release.
        """
    @property
    def portions(self, /) -> int: ...
    @property
    def released_ms(self, /) -> int | None:
        """
        When the owner released it ([`UpdateKind::Released`]), in Unix
        milliseconds. A lost feed holds new feeds back until it settles or
        is released, however long that takes; once released it is kept open
        to a late end for a week from then.
        """
    @property
    def reported(self, /) -> bool:
        """
        Reported still unknown ([`UpdateKind::StillUnknown`]): it is not
        reported again.
        """
    @property
    def sent_ms(self, /) -> int:
        """
        When it went out, in Unix milliseconds: it is reported still
        unknown once the settle window has passed since, if nothing showed
        sooner that it never ran.
        """

@final
class Plan:
    """
    One feeding plan, by its slot.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def days(self, /) -> int:
        """
        The weekdays it runs, Monday in bit 0; 0 for a plan that runs once.
        """
    @property
    def enabled(self, /) -> bool: ...
    @property
    def minute(self, /) -> int:
        """
        The minute of the feeder's local day it is due at (0-1439).
        """
    @property
    def portions(self, /) -> int: ...
    @property
    def slot(self, /) -> int: ...

@final
class PortionsToday:
    """
    The portions fed on the feeder's local day: a range, since an
    uncertain feed is never one number.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def at_most(self, /) -> int: ...
    @property
    def confirmed(self, /) -> int: ...
    @property
    def feeds(self, /) -> int: ...

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
    NewerBuild: Final[Readiness]
    """
    A value a newer feeder build added, which this integration
    cannot name: update it.
    """
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
class Redeemed:
    """
    A key an invite enrolled: its secret and its feeder, how it joined, and
    the key still held for the next `redeem`, if any.
    """
    @property
    def feeder(self, /) -> bytes: ...
    @property
    def held(self, /) -> HeldKey | None: ...
    @property
    def joined(self, /) -> Joined: ...
    @property
    def secret(self, /) -> bytes: ...

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
    NewerBuild: Final[RefusalKind]
    """
    A value a newer feeder build added, which this integration
    cannot name: update it.
    """
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
    NewerBuild: Final[Role]
    """
    A role a newer build added: it grants nothing here.
    """
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
class Settings:
    """
    The settings Home Assistant shows and changes.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def desiccant_days(self, /) -> int:
        """
        The desiccant reminder's period, 1-90 days.
        """
    @property
    def lights(self, /) -> bool: ...
    @property
    def meal_reminder_minutes(self, /) -> int | None:
        """
        Minutes ahead of each meal its `meal_soon` event comes; `None` when
        off, or when it is a kind of reminder a newer build added.
        """
    @property
    def sound(self, /) -> bool: ...
    @property
    def volume(self, /) -> int:
        """
        0-100.
        """

@final
class Skew:
    """
    How the feeder's device API revision stands against this
    integration's. Either way the session reads and writes everything
    both know; this says which side to update for the rest.
    """
    FeederAhead: Final[Skew]
    """
    The feeder is newer: what it added reads as `newer_build` until
    this integration is updated.
    """
    FeederBehind: Final[Skew]
    """
    The feeder is older: what it lacks is refused until it is
    updated, from the app.
    """
    Legacy: Final[Skew]
    """
    The feeder runs a build from before API revisions, which takes
    nothing this integration reads or writes: it must be updated,
    from the app, before the session can serve it.
    """
    NewerBuild: Final[Skew]
    """
    A value a newer build added, which this integration cannot name.
    """
    Same: Final[Skew]
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
class State:
    """
    Everything the session knows of the feeder now.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def last_fed(self, /) -> Feed | None:
        """
        The newest feed the history has given.
        """
    @property
    def next_meal(self, /) -> NextMeal | None: ...
    @property
    def plans(self, /) -> list[Plan] | None:
        """
        The filled plan slots, by slot; `None` until read or on a feeder
        without plans.
        """
    @property
    def portion_mg(self, /) -> int | None:
        """
        The weight of one portion, milligrams; `None` until measured, or
        for a client that does not read the calibration.
        """
    @property
    def portions_today(self, /) -> PortionsToday | None:
        """
        `None` while the feeder's clock is invalid.
        """
    @property
    def settings(self, /) -> Settings | None:
        """
        `None` until read.
        """
    @property
    def status(self, /) -> Status: ...

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
    def readiness(self, /) -> Readiness:
        """
        Whether a feed can start now.
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
class Tune:
    """
    A tune the feeder plays on request.
    """
    Alert: Final[Tune]
    Chime: Final[Tune]
    Coin: Final[Tune]
    Meal: Final[Tune]
    Test: Final[Tune]
    Twinkle: Final[Tune]
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
class Update:
    """
    What the session tells Home Assistant, in order.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def alerts(self, /) -> Alerts | None:
        """
        For [`UpdateKind::Alerts`].
        """
    @property
    def answered(self, /) -> list[int]:
        """
        For [`UpdateKind::Answered`]: every feed number to keep, in place of
        those kept before.
        """
    @property
    def build(self, /) -> str | None:
        """
        For [`UpdateKind::Connected`]: the build that answered, this
        client's role, what the feeder serves and how its API revision
        stands against this integration's.
        """
    @property
    def capabilities(self, /) -> Capabilities | None: ...
    @property
    def event(self, /) -> Event | None:
        """
        For [`UpdateKind::Event`].
        """
    @property
    def feed(self, /) -> Feed | None:
        """
        For [`UpdateKind::Settled`]: the feed whose answer never came.
        """
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
    def pending(self, /) -> Pending | None:
        """
        For [`UpdateKind::Pending`]: the feed to keep. For
        [`UpdateKind::Settled`], [`UpdateKind::StillUnknown`], [`UpdateKind::Released`],
        [`UpdateKind::Forgotten`], [`UpdateKind::Sending`] and
        [`UpdateKind::Cleared`]: the pending feed it accounts for.
        """
    @property
    def portions(self, /) -> int | None:
        """
        For [`UpdateKind::StillUnknown`]: the portions that feed asked for.
        """
    @property
    def reason(self, /) -> str | None:
        """
        Why the session ended, for [`UpdateKind::Disconnected`]: for logs.
        """
    @property
    def replayed(self, /) -> bool:
        """
        Whether the event came from the history read at connect: already
        part of the state, and not news.
        """
    @property
    def role(self, /) -> Role | None: ...
    @property
    def skew(self, /) -> Skew | None: ...
    @property
    def state(self, /) -> State | None:
        """
        For [`UpdateKind::State`].
        """

@final
class UpdateKind:
    """
    What a session update carries.
    """
    Alerts: Final[UpdateKind]
    """
    The feeder's alerts moved: what is open, and what opened and
    cleared since.
    """
    Answered: Final[UpdateKind]
    """
    This client has the end of one more feed number: its write was
    answered, or its `Fed` settled a lost feed. `answered` lists
    every number to keep for the next session, where a `Fed` for one
    of them settles nothing, in place of those kept before.
    """
    BowlsChecked: Final[UpdateKind]
    """
    The owner checked the bowls after the integration's store did
    not read: feeds go again.
    """
    Cleared: Final[UpdateKind]
    """
    A feed kept while it went out has its answer, or never went: no
    longer kept. A feed it ran comes as [`UpdateKind::Answered`]
    first.
    """
    Connected: Final[UpdateKind]
    """
    A session opened: its build, role and capabilities.
    """
    Disconnected: Final[UpdateKind]
    Event: Final[UpdateKind]
    """
    An event; `replayed` when it came from the history read at
    connect rather than as it happened.
    """
    Forgotten: Final[UpdateKind]
    """
    A feed whose answer never came, reported still unknown, waited
    as long as one is kept with no end found: no longer pending, and
    nothing settles it now.
    """
    Gap: Final[UpdateKind]
    """
    Events this session never received: what happened meanwhile is
    not known.
    """
    Pending: Final[UpdateKind]
    """
    A feed's answer never came: it is pending until a later update
    settles it, and comes before any update that does.
    """
    Released: Final[UpdateKind]
    """
    The owner released a lost feed, in `pending` with when: it holds
    new feeds back no longer, and stays pending, so a later update
    may still settle it, until [`UpdateKind::Forgotten`].
    """
    Revoked: Final[UpdateKind]
    """
    The feeder no longer knows this client's key; the session will
    not reconnect.
    """
    Sending: Final[UpdateKind]
    """
    A feed about to go out, in `pending`: keep it as pending, with no
    repair yet, and say whether the store saved it (`Feeder.saved` or
    `Feeder.unsaved`): it goes only once saved. A restart before its
    answer finds it lost.
    """
    Settled: Final[UpdateKind]
    """
    A feed whose answer never came ran: its `Fed` event says how.
    """
    State: Final[UpdateKind]
    """
    What the session knows of the feeder changed.
    """
    StillUnknown: Final[UpdateKind]
    """
    A feed whose answer never came has no `Fed` event this session
    could find yet: whether it ran is not known. It stays pending,
    and holds new feeds back until a later update settles it or the
    owner releases it ([`UpdateKind::Released`]).
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
    It never went out; `not_sent` says why.
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
    def not_sent(self, /) -> NotSent | None:
        """
        Why it never went out, for [`WriteResult::NotSent`].
        """
    @property
    def pending(self, /) -> Pending | None:
        """
        For an uncertain feed: what to keep until an update settles it.
        """
    @property
    def refusal(self, /) -> RefusalKind | None:
        """
        Why it refused, for [`WriteResult::Refused`].
        """
    @property
    def result(self, /) -> WriteResult: ...

def invite_feeder(text: str) -> bytes:
    """
    The identity key of the feeder `text` invites to.
    """

def public_key(secret: bytes) -> bytes:
    """
    The public key of `secret`, as the feeder lists this client.
    """

def redeem(invite: str, label: str, address: str | None = None, held: HeldKey | None = None) -> Redeemed:
    """
    Join with `invite`, listed as `label`, keeping the key only as a member:
    any other role is revoked at once. `held` is the key an earlier unchecked
    join left: for this invite's feeder it is checked first, and the invite,
    which works once, is spent only when the feeder does not know it. A join
    that did not enroll a key raises `JoinFailed`, whose `held` is the key
    to hand the next call.
    """
