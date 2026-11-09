from _typeshed import SupportsGetItem
from typing import Final, SupportsFloat, SupportsIndex, final
from typing_extensions import disjoint_base

__all__ = ["AccountRole", "By", "Capabilities", "Event", "EventKind", "Feed", "FeedFault", "FeedOutcome", "FeedReason", "Feeder", "FoodLevel", "Joined", "Login", "NextMeal", "NotSent", "Pending", "Plan", "PortionsToday", "Readiness", "RefusalKind", "Role", "ServerFeeder", "ServerProblem", "ServerRefused", "Settings", "Skew", "State", "Status", "Tune", "Update", "UpdateKind", "WellfedError", "WriteResult", "Written", "generate_secret", "invite_feeder", "join", "public_key", "server_client_listed", "server_feeders", "server_login", "server_revoke_client", "server_revoke_token"]

@final
class AccountRole:
    """
    An account's role on the owner's server.
    """
    Member: Final[AccountRole]
    NewerBuild: Final[AccountRole]
    """
    A role a newer server added: it grants nothing here.
    """
    Owner: Final[AccountRole]
    Viewer: Final[AccountRole]
    """
    Reads, and changes nothing: Home Assistant refuses it at setup.
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
    def __new__(cls, /, secret: bytes, identity: bytes, address: str | None = None, pending: SupportsGetItem[int, Pending] = ..., answered: SupportsGetItem[int, SupportsIndex] = ...) -> Feeder:
        """
        `pending` and `answered` are what the last run left: the feeds
        still unsettled, and the feed numbers answered since the oldest.
        """
    def cancel_feed(self, /, feed: SupportsIndex) -> Written:
        """
        Stop the feed the feeder numbered `feed`, if it still runs.
        """
    def feed(self, /, portions: SupportsIndex) -> Written:
        """
        Start a feed; how it ended. Never retry an uncertain one.
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
    def reset_desiccant(self, /) -> Written:
        """
        Record a fresh desiccant pack today.
        """
    @staticmethod
    def server(url: str, token: str, feeder_id: str, identity: bytes, pending: SupportsGetItem[int, Pending] = ...) -> Feeder:
        """
        The session to the feeder `feeder_id` on the owner's server at
        `url`, signed in with `token`; `identity` is the feeder's, and a
        feeder under that id with another one is never served as this one.
        `pending` is what the last run left unsettled.
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
class Login:
    """
    A sign-in that holds: Home Assistant's own token on the server.
    """
    @property
    def role(self, /) -> AccountRole: ...
    @property
    def token(self, /) -> str:
        """
        The bearer token, shown once; the config entry keeps it.
        """
    @property
    def token_id(self, /) -> str:
        """
        Its id, to revoke it by.
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
    Invalid: Final[NotSent]
    """
    A value outside what the feeder accepts.
    """
    Legacy: Final[NotSent]
    """
    The feeder runs a build from before API revisions, which the
    server sends nothing but an update: update the feeder.
    """
    NotRead: Final[NotSent]
    """
    What the change is based on has not been read yet.
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
    A feed whose answer never came, until it is settled: what the
    integration keeps across a restart, and hands back to the next session
    so it is settled then.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    def __new__(cls, /, id: str, portions: SupportsIndex, sent_ms: SupportsIndex, after: SupportsIndex | None = None) -> Pending:
        """
        A pending feed as the integration kept it, to hand back to a new
        session.
        """
    @property
    def after(self, /) -> int | None:
        """
        Directly: the newest event known when it went out, which its `Fed`
        comes after.
        """
    @property
    def id(self, /) -> str:
        """
        Names it: through the owner's server, the idempotency key it went
        under, which the server's journal answers by.
        """
    @property
    def portions(self, /) -> int: ...
    @property
    def sent_ms(self, /) -> int:
        """
        When it went out, in Unix milliseconds: it is still unknown once
        the settle window has passed since.
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
    The role the feeder enrolled this client with, or through the
    owner's server the account's role there.
    """
    Member: Final[Role]
    NewerBuild: Final[Role]
    """
    A role a newer build added: it grants nothing here.
    """
    Owner: Final[Role]
    Server: Final[Role]
    UpdateSigner: Final[Role]
    Viewer: Final[Role]
    """
    Through the owner's server, an account that reads and changes
    nothing.
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
class ServerFeeder:
    """
    A feeder the server holds, as the setup flow offers it.
    """
    def __eq__(self, value: object, /) -> bool: ...
    def __ne__(self, value: object, /) -> bool: ...
    @property
    def id(self, /) -> str:
        """
        Its id on the server.
        """
    @property
    def identity(self, /) -> bytes:
        """
        Its identity key, which names it on either route.
        """
    @property
    def name(self, /) -> str: ...

@final
class ServerProblem:
    """
    Why a call to the owner's server at setup failed, as the flow words
    it.
    """
    AccountDisabled: Final[ServerProblem]
    """
    The account is past its expiry.
    """
    BadCredentials: Final[ServerProblem]
    """
    The name or password is wrong.
    """
    Forbidden: Final[ServerProblem]
    """
    The account's role may not do that.
    """
    LoginLimited: Final[ServerProblem]
    """
    Too many failed logins: wait a minute.
    """
    NotAServer: Final[ServerProblem]
    """
    Something answered, but not as a Wellfed server of this API.
    """
    Redirected: Final[ServerProblem]
    """
    The address redirects elsewhere: use the address it names.
    """
    Refused: Final[ServerProblem]
    """
    The server refused for another reason.
    """
    Unauthorized: Final[ServerProblem]
    """
    The token no longer holds: log in again.
    """
    Unreachable: Final[ServerProblem]
    """
    Nothing answered at that address.
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
class ServerRefused(WellfedError):
    """
    The owner's server refused a setup call, or could not be reached;
    `problem` says how, as the setup flow words it.
    """
    def __new__(cls, /, problem: ServerProblem, *_args) -> ServerRefused: ...
    @property
    def problem(self, /) -> ServerProblem: ...

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
    ServerAhead: Final[Skew]
    """
    Through the owner's server: the server is newer than this
    integration, so update the integration.
    """
    ServerBehind: Final[Skew]
    """
    Through the owner's server: the server is older than the feeder
    or this integration, so update the server.
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
        For [`UpdateKind::Settled`], [`UpdateKind::SettledUnfed`] and
        [`UpdateKind::StillUnknown`]: the pending feed it accounts for.
        """
    @property
    def portions(self, /) -> int | None:
        """
        For [`UpdateKind::StillUnknown`]: the portions that feed asked for.
        """
    @property
    def reason(self, /) -> str | None:
        """
        Why the session ended, for [`UpdateKind::Disconnected`] and
        [`UpdateKind::ServerLostFeeder`]: for logs.
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
    Gap: Final[UpdateKind]
    """
    Events this session never received: what happened meanwhile is
    not known.
    """
    Revoked: Final[UpdateKind]
    """
    The feeder no longer knows this client's key; the session will
    not reconnect.
    """
    ServerLostFeeder: Final[UpdateKind]
    """
    The server no longer reaches the feeder for good: the feeder
    dropped the server's key, or the server no longer holds it.
    The owner adopts it again.
    """
    Settled: Final[UpdateKind]
    """
    A feed whose answer never came ran: its `Fed` event, or the
    server's journal, says how.
    """
    SettledUnfed: Final[UpdateKind]
    """
    A feed whose answer never came did not run: the feeder refused
    it, as the server's journal learned.
    """
    State: Final[UpdateKind]
    """
    What the session knows of the feeder changed.
    """
    StillUnknown: Final[UpdateKind]
    """
    A feed whose answer never came has no `Fed` event this session
    could find: whether it ran is not known.
    """
    TokenRefused: Final[UpdateKind]
    """
    The server no longer takes this integration's token (revoked,
    expired, or its account disabled); the session will not
    reconnect until a new login.
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

def generate_secret() -> bytes:
    """
    A new client key's secret, for the config entry.
    """

def invite_feeder(text: str) -> bytes:
    """
    The identity key of the feeder `text` invites to.
    """

def join(secret: bytes, invite: str, label: str, address: str | None = None) -> Joined:
    """
    Redeem `invite` as the key `secret`, listed as `label`, keeping the key
    only as a member: any other role is revoked at once. Runs once: an invite
    is single-use.
    """

def public_key(secret: bytes) -> bytes:
    """
    The public key of `secret`, as the feeder lists this client.
    """

def server_client_listed(url: str, token: str, feeder_id: str, key: bytes) -> bool | None:
    """
    Whether the feeder `feeder_id` still lists the client key `key`, as the
    server reads it now; `None` when the server cannot say (the feeder is
    offline, or the account may not list its clients).
    """

def server_feeders(url: str, token: str) -> list[ServerFeeder]:
    """
    The feeders the server at `url` holds, as `token` sees them.
    """

def server_login(url: str, name: str, password: str, label: str) -> Login:
    """
    Sign in to the owner's server at `url` as `name`, and create a token
    labelled `label` for Home Assistant. The password is used for this alone
    and not kept; an account that may change nothing is refused.
    """

def server_revoke_client(url: str, token: str, feeder_id: str, key: bytes) -> bool:
    """
    Remove the client key `key` from the feeder `feeder_id` through the
    server; whether it was done. Sent once: `False` when it was refused, or
    may not have run.
    """

def server_revoke_token(url: str, token: str, token_id: str) -> None:
    """
    Revoke the token `token_id` with itself.
    """
