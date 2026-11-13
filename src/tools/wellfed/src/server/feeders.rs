//! The device verbs over the server: the feeders it holds, their status,
//! settings, lock, desiccant reminder, feeds and history, and the journal
//! its writes go through.
//!
//! Each write is sent once under a fresh key ([`super::write`]). A feed is
//! answered once the feeder has reserved it, and its result is the journal
//! entry the feeder's history settles; the command follows the stream for
//! it, opened before the feed is sent so the result cannot slip past.

use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use uuid::Uuid;
use wellfed_server_client::api::{
    Command, CommandOutcome, CommandQuery, CommandState, CommandStateFilter, EventPage, EventQuery,
    Feeder, FeederId, FeederSettingsPatch, MealReminder, StreamEvent, StreamQuery, Window,
};
use wellfed_server_client::{Client, Lock, Updates};

use super::{Output, Settled, Written, text, write};
use crate::Toggle;

/// Longer than any feed the owner's limits allow, as the device API's
/// `feed` waits.
const FEED_WAIT: Duration = Duration::from_secs(120);

/// Which feeder: by its id or its name, or the only one.
#[derive(Args)]
pub(crate) struct Which {
    /// The feeder's id or name; needed when the server holds more than one.
    #[arg(long)]
    feeder: Option<String>,
}

impl Which {
    /// The feeder this names, from the server's list.
    pub(super) async fn id(&self, client: &Client) -> Result<FeederId> {
        let feeders = client.feeders().await?;
        let matching: Vec<&Feeder> = feeders
            .iter()
            .filter(|feeder| {
                self.feeder.as_deref().is_none_or(|which| {
                    feeder.name == which || feeder.id.to_string() == which.to_lowercase()
                })
            })
            .collect();

        let [feeder] = matching.as_slice() else {
            match &self.feeder {
                Some(which) => bail!("{} feeders are named or numbered {which:?}", matching.len()),
                None => bail!(
                    "the server holds {} feeders; pass --feeder with an id or a name",
                    matching.len()
                ),
            }
        };
        Ok(feeder.id)
    }
}

#[derive(Args)]
pub(crate) struct SettingsArgs {
    #[command(flatten)]
    which: Which,
    #[arg(long)]
    lights: Option<Toggle>,
    #[arg(long)]
    sound: Option<Toggle>,
    /// 0-100.
    #[arg(long)]
    volume: Option<u8>,
    /// When lit lights may show: `always`, or `HH:MM-HH:MM` on the
    /// feeder's local clock (inclusive; may wrap midnight).
    #[arg(long, value_name = "WINDOW")]
    lights_window: Option<Window>,
    /// When sounds may play: `always`, or `HH:MM-HH:MM` as for
    /// `--lights-window`.
    #[arg(long, value_name = "WINDOW")]
    sound_window: Option<Window>,
    /// Days from a desiccant replacement until the next is due (1-90).
    #[arg(long, value_name = "DAYS", value_parser = crate::configure::desiccant_days)]
    desiccant_days: Option<u8>,
    /// A `meal_soon` event this many minutes (1-60) before each scheduled
    /// meal, or `off`.
    #[arg(long, value_name = "off|MINUTES")]
    meal_reminder: Option<MealReminder>,
}

#[derive(Args)]
pub(crate) struct DesiccantArgs {
    #[command(flatten)]
    which: Which,
    /// Record that the desiccant was replaced today (by the feeder's
    /// clock, which must be valid).
    #[arg(long)]
    reset: bool,
}

#[derive(Args)]
pub(crate) struct LockArgs {
    #[command(flatten)]
    which: Which,
    state: Toggle,
}

#[derive(Args)]
pub(crate) struct FeedArgs {
    #[command(flatten)]
    which: Which,
    /// Portions to dispense, 1-48.
    #[arg(long, default_value_t = 1, value_parser = crate::plans::portions())]
    portions: u8,
}

#[derive(Args)]
pub(crate) struct CancelFeedArgs {
    #[command(flatten)]
    which: Which,
    /// The feed's number, as `wellfed server feed` printed it.
    feed: u32,
}

#[derive(Args)]
pub(crate) struct EventsArgs {
    #[command(flatten)]
    which: Which,
    /// The feeder's epoch (default: its current one).
    #[arg(long)]
    epoch: Option<u32>,
    /// Start after this sequence number (default: the oldest held).
    #[arg(long)]
    after: Option<u64>,
    /// At most this many (1-1000, default 100).
    #[arg(long)]
    limit: Option<u32>,
    /// Then keep following the stream and print new events as they come.
    #[arg(long)]
    follow: bool,
}

#[derive(Args)]
pub(crate) struct CommandArgs {
    /// The write's key, as the command that sent it printed.
    key: Uuid,
}

#[derive(Args)]
pub(crate) struct CommandsArgs {
    /// One feeder's writes only.
    #[arg(long)]
    feeder: Option<String>,
    /// Writes in this state only: `unknown` is the uncertain list.
    #[arg(long, value_enum)]
    state: Option<State>,
    /// The previous page's `next`, as it came.
    #[arg(long)]
    after: Option<String>,
    /// At most this many (1-1000, default 100).
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum State {
    Sent,
    Answered,
    Unknown,
    Resolved,
}

impl State {
    const fn api(self) -> CommandStateFilter {
        match self {
            Self::Sent => CommandStateFilter::Sent,
            Self::Answered => CommandStateFilter::Answered,
            Self::Unknown => CommandStateFilter::Unknown,
            Self::Resolved => CommandStateFilter::Resolved,
        }
    }
}

pub(super) async fn list(client: &Client, out: Output) -> Result<Settled> {
    let feeders = client.feeders().await?;
    out.show(&feeders, |feeders| text::lines(feeders, text::feeder))?;
    Ok(Settled::Known)
}

pub(super) async fn status(client: &Client, which: &Which, out: Output) -> Result<Settled> {
    let id = which.id(client).await?;
    let detail = client.feeder(id).await?;
    out.show(&detail, text::status)?;
    Ok(Settled::Known)
}

/// Without a change, the settings as the feeder holds them; with one, the
/// change against the generation just read, so a change made meanwhile is
/// refused (the current settings print) rather than overwritten.
pub(super) async fn settings(client: &Client, args: SettingsArgs, out: Output) -> Result<Settled> {
    let id = args.which.id(client).await?;
    let current = client.settings(id).await?;
    let patch = FeederSettingsPatch {
        lights: args.lights.map(Toggle::on),
        sound: args.sound.map(Toggle::on),
        volume: args.volume,
        lights_window: args.lights_window,
        sound_window: args.sound_window,
        desiccant_days: args.desiccant_days,
        meal_reminder: args.meal_reminder,
    };
    if patch == FeederSettingsPatch::default() {
        out.show(&current, text::settings)?;
        return Ok(Settled::Known);
    }

    let key = Uuid::new_v4();
    let Some(base) = current.generation() else {
        bail!("{}", crate::text::UPDATE_FOR_DEVICE.trim_end());
    };
    let sent = client.put_settings(id, base, &patch, key);
    let Written::Answered(settings) = write(out, key, sent).await? else {
        return Ok(Settled::Unknown);
    };
    out.show(&settings, text::settings)?;
    Ok(Settled::Known)
}

pub(super) async fn lock(client: &Client, args: LockArgs, out: Output) -> Result<Settled> {
    let id = args.which.id(client).await?;
    let lock = if args.state.on() {
        Lock::Locked
    } else {
        Lock::Unlocked
    };

    let key = Uuid::new_v4();
    let Written::Answered(()) = write(out, key, client.set_lock(id, lock, key)).await? else {
        return Ok(Settled::Unknown);
    };
    out.text(match lock {
        Lock::Locked => "The panel is locked.",
        Lock::Unlocked => "The panel is unlocked.",
    });
    Ok(Settled::Known)
}

/// The reminder from the feeder's status; with `--reset`, a replacement
/// recorded today first, under a fresh key.
pub(super) async fn desiccant(
    client: &Client,
    args: DesiccantArgs,
    out: Output,
) -> Result<Settled> {
    let id = args.which.id(client).await?;
    if args.reset {
        let key = Uuid::new_v4();
        let sent = client.reset_desiccant(id, key);
        let Written::Answered(()) = write(out, key, sent).await? else {
            return Ok(Settled::Unknown);
        };
        out.text("Desiccant replaced today.");
    }

    let detail = client.feeder(id).await?;
    out.show(&detail, text::desiccant)?;
    Ok(Settled::Known)
}

/// One feed under a fresh key, sent once: the entry the server answers
/// once the feeder has reserved it, then the entry its result settles.
pub(super) async fn feed(client: &Client, args: FeedArgs, out: Output) -> Result<Settled> {
    let id = args.which.id(client).await?;
    let mut updates = client.stream(&StreamQuery::default(), None).await?;

    let key = Uuid::new_v4();
    let sent = client.feed(id, args.portions, key);
    let Written::Answered(entry) = write(out, key, sent).await? else {
        return Ok(Settled::Unknown);
    };
    out.show(&entry, |entry| match entry.feed {
        Some(feed) => format!("Feed {feed} started (key {}).", entry.key),
        None => format!("Feed sent (key {}).", entry.key),
    })?;
    if entry.state == CommandState::Resolved {
        return settle(&entry, out);
    }

    let result = tokio::time::timeout(FEED_WAIT, resolved(&mut updates, key)).await;
    match result {
        Ok(Some(entry)) => settle(&entry, out),
        Ok(None) | Err(_) => {
            out.text(format!("No result yet after {} s.", FEED_WAIT.as_secs()));
            out.unknown(key, None)?;
            Ok(Settled::Unknown)
        }
    }
}

/// The entry under `key` once the stream reports it resolved; `None` if
/// the stream ends first.
async fn resolved(updates: &mut Updates, key: Uuid) -> Option<Command> {
    while let Some(update) = updates.next().await {
        let Ok(update) = update else {
            return None;
        };
        if let StreamEvent::Command { command } = update.event
            && command.key == key
            && command.state == CommandState::Resolved
        {
            return Some(command);
        }
    }
    None
}

pub(super) async fn cancel_feed(
    client: &Client,
    args: CancelFeedArgs,
    out: Output,
) -> Result<Settled> {
    let id = args.which.id(client).await?;

    let key = Uuid::new_v4();
    let sent = client.cancel_feed(id, args.feed, key);
    let Written::Answered(()) = write(out, key, sent).await? else {
        return Ok(Settled::Unknown);
    };
    out.text(format!("Feed {} is stopping.", args.feed));
    Ok(Settled::Known)
}

/// A page of the feeder's history, then with `--follow` the rest of it
/// and its new events and gaps from the stream.
pub(super) async fn events(client: &Client, args: EventsArgs, out: Output) -> Result<Settled> {
    let id = args.which.id(client).await?;
    // Open first, so nothing recorded between the history and the stream
    // is missed: the stream starts live, and the pages reach up to it.
    let mut updates = match args.follow {
        true => Some(client.stream(&StreamQuery::default(), None).await?),
        false => None,
    };

    let query = EventQuery {
        epoch: args.epoch,
        after: args.after,
        limit: args.limit,
        kind: None,
    };
    let Some(updates) = updates.as_mut() else {
        let page = page(client, id, &query, out).await?;
        if let Some(next) = page.next {
            eprintln!("More after {next}: --after {next}");
        }
        return Ok(Settled::Known);
    };

    // The stream opened before the history was read, so it may repeat the
    // history's newest events.
    let mut printed = history(client, id, query, out, None).await?;
    while let Some(update) = updates.next().await {
        let update = update?;
        match followed(&update.event, id, printed, &query) {
            Followed::Show(text) => {
                out.show(&update.event, |_| text)?;
                if let StreamEvent::Event { event, .. } = &update.event {
                    printed = Some((event.epoch, event.seq));
                }
            }
            Followed::Read(skipped) => {
                eprintln!("The stream fell behind; reading what it skipped.");
                printed = history(client, id, skipped, out, printed).await?;
            }
            Followed::Pass => {}
        }
    }
    bail!("the server ended the stream")
}

/// What `--follow` does with one update of the stream.
#[derive(Debug, PartialEq, Eq)]
enum Followed {
    /// Show it, as this text.
    Show(String),
    /// Read the history from this query: the server dropped what this
    /// subscriber fell behind on.
    Read(EventQuery),
    /// Another feeder's, one already shown, or not an event.
    Pass,
}

/// What to do with `event`, for feeder `id`, after the newest event shown
/// (`printed`), in a run that began at `start`. A resync reads on from the
/// newest shown, or from where the run began when it showed none.
fn followed(
    event: &StreamEvent,
    id: FeederId,
    printed: Option<(u32, u64)>,
    start: &EventQuery,
) -> Followed {
    match event {
        StreamEvent::Event { feeder, event }
            if *feeder == id && Some((event.epoch, event.seq)) > printed =>
        {
            Followed::Show(text::event(event))
        }
        StreamEvent::Gap { feeder, gap } if *feeder == id => Followed::Show(text::gap(gap)),
        StreamEvent::Resync => Followed::Read(EventQuery {
            epoch: printed.map(|(epoch, _)| epoch).or(start.epoch),
            after: printed.map(|(_, seq)| seq).or(start.after),
            limit: start.limit,
            kind: None,
        }),
        _ => Followed::Pass,
    }
}

/// One page of history from `query`, shown.
async fn page(client: &Client, id: FeederId, query: &EventQuery, out: Output) -> Result<EventPage> {
    let page = client.events(id, query).await?;
    out.show(&page, |page| {
        let mut lines: Vec<String> = page.events.iter().map(text::event).collect();
        lines.extend(page.gaps.iter().map(text::gap));
        lines.join("\n")
    })?;
    Ok(page)
}

/// Every page of history from `query` to the newest, shown in order; the
/// newest event shown since `printed`.
async fn history(
    client: &Client,
    id: FeederId,
    mut query: EventQuery,
    out: Output,
    mut printed: Option<(u32, u64)>,
) -> Result<Option<(u32, u64)>> {
    loop {
        let page = page(client, id, &query, out).await?;
        if let Some(event) = page.events.last() {
            printed = printed.max(Some((event.epoch, event.seq)));
        }
        let Some(next) = page.next else {
            return Ok(printed);
        };
        query.after = Some(next);
        query.epoch = page.epoch.or(query.epoch);
    }
}

/// The journal entry under a key: how a write whose outcome was unknown
/// is settled. Exits as the write would have: 3 while its outcome is
/// still unknown.
pub(super) async fn command(client: &Client, args: &CommandArgs, out: Output) -> Result<Settled> {
    let entry = client.command(args.key).await?;
    out.show(&entry, text::command)?;
    settled(&entry)
}

pub(super) async fn commands(client: &Client, args: CommandsArgs, out: Output) -> Result<Settled> {
    let feeder = match args.feeder {
        Some(feeder) => Some(
            Which {
                feeder: Some(feeder),
            }
            .id(client)
            .await?,
        ),
        None => None,
    };
    let query = CommandQuery {
        feeder,
        state: args.state.map(State::api),
        after: args.after,
        limit: args.limit,
    };

    let page = client.commands(&query).await?;
    out.show(&page, |page| text::lines(&page.items, text::command))?;
    if let Some(next) = &page.next {
        eprintln!("More after {next}: --after {next}");
    }
    Ok(Settled::Known)
}

/// A settled feed's result, printed: done is success, an uncertain or
/// unrecorded feed exits 3, and any other end is the command's failure.
fn settle(entry: &Command, out: Output) -> Result<Settled> {
    out.show(entry, |entry| {
        entry
            .outcome
            .as_ref()
            .map_or_else(String::new, text::outcome)
    })?;
    settled(entry)
}

/// What an entry says of its write, as an exit status: a refusal or a feed
/// that ended without its portions is an error; a write not yet answered,
/// a feed still running, and an uncertain or unrecorded feed are unknown.
fn settled(entry: &Command) -> Result<Settled> {
    let pending = matches!(entry.state, CommandState::Sent | CommandState::Unknown);
    match &entry.outcome {
        _ if pending => Ok(Settled::Unknown),
        // An outcome a newer server added says no more than an unknown one.
        None
        | Some(
            CommandOutcome::Feeding { .. } | CommandOutcome::Unrecorded | CommandOutcome::Unknown,
        ) => Ok(Settled::Unknown),
        Some(
            CommandOutcome::Done
            | CommandOutcome::Settings { .. }
            | CommandOutcome::Plans { .. }
            | CommandOutcome::Calibration { .. }
            | CommandOutcome::Call { .. }
            | CommandOutcome::Invited { .. },
        ) => Ok(Settled::Known),
        Some(CommandOutcome::Superseded) => {
            bail!("another change to the meal call came first: nothing was sent again")
        }
        Some(CommandOutcome::Refused { refusal }) => {
            bail!("the feeder refused it: {}", text::word(refusal))
        }
        Some(CommandOutcome::Fed { outcome, .. }) => crate::feeding::settled(*outcome),
    }
}

#[cfg(test)]
mod tests {
    use wellfed_server_client::api::{
        CommandKind, CoreEvent, EventBody, FeedOutcome, FeedReason, Gap, HistoryEvent, UserId,
    };

    use super::*;

    /// With `--follow`, an event is shown once, after what was shown; and
    /// when the server says the stream fell behind, the history is read on
    /// from the newest event shown, or from where the run began.
    #[test]
    fn a_resync_reads_the_history_on_from_the_newest_event_shown() {
        let (id, other) = (FeederId(Uuid::from_u128(1)), FeederId(Uuid::from_u128(2)));
        let start = EventQuery {
            epoch: Some(2),
            after: Some(10),
            limit: Some(50),
            kind: None,
        };
        let event = |feeder, seq| StreamEvent::Event {
            feeder,
            event: HistoryEvent {
                epoch: 3,
                seq,
                at: None,
                received: jiff::Timestamp::UNIX_EPOCH,
                body: EventBody::Core(CoreEvent::Boot {
                    bank: 1,
                    reset_flags: 0,
                }),
            },
        };

        let shown = Some((3, 40));
        assert!(matches!(
            followed(&event(id, 41), id, shown, &start),
            Followed::Show(_)
        ));
        for passed in [event(id, 40), event(id, 39), event(other, 99)] {
            assert_eq!(followed(&passed, id, shown, &start), Followed::Pass);
        }
        let gap = StreamEvent::Gap {
            feeder: id,
            gap: Gap {
                epoch: 3,
                from: 41,
                to: 44,
                received: jiff::Timestamp::UNIX_EPOCH,
            },
        };
        assert!(matches!(
            followed(&gap, id, shown, &start),
            Followed::Show(_)
        ));

        assert_eq!(
            followed(&StreamEvent::Resync, id, shown, &start),
            Followed::Read(EventQuery {
                epoch: Some(3),
                after: Some(40),
                limit: Some(50),
                kind: None,
            })
        );
        assert_eq!(
            followed(&StreamEvent::Resync, id, None, &start),
            Followed::Read(start),
            "nothing shown yet: from where the run began"
        );
    }

    fn entry(state: CommandState, outcome: Option<CommandOutcome>) -> Command {
        Command {
            key: Uuid::nil(),
            user: UserId(Uuid::nil()),
            token: None,
            feeder: FeederId(Uuid::nil()),
            kind: CommandKind::Feed,
            state,
            outcome,
            feed: Some(1),
            boot_count: None,
            sent: jiff::Timestamp::UNIX_EPOCH,
            updated: jiff::Timestamp::UNIX_EPOCH,
        }
    }

    fn fed(outcome: FeedOutcome) -> Option<CommandOutcome> {
        Some(CommandOutcome::Fed {
            feed: 1,
            requested: 2,
            completed: 2,
            outcome,
            reason: FeedReason::Done,
        })
    }

    #[test]
    fn an_entry_exits_as_its_write_settled() {
        use CommandState::{Answered, Resolved, Sent, Unknown};

        let known = [
            entry(Answered, Some(CommandOutcome::Done)),
            entry(Resolved, fed(FeedOutcome::Done)),
        ];
        for entry in known {
            assert_eq!(settled(&entry).ok(), Some(Settled::Known), "{entry:?}");
        }

        let unknown = [
            entry(Sent, None),
            entry(Unknown, None),
            entry(Answered, Some(CommandOutcome::Feeding { feed: 1 })),
            entry(Resolved, fed(FeedOutcome::Uncertain)),
            entry(Resolved, Some(CommandOutcome::Unrecorded)),
        ];
        for entry in unknown {
            assert_eq!(settled(&entry).ok(), Some(Settled::Unknown), "{entry:?}");
        }

        let failed = [
            entry(Resolved, fed(FeedOutcome::Failed)),
            entry(Resolved, fed(FeedOutcome::Cancelled)),
            entry(
                Answered,
                Some(CommandOutcome::Refused {
                    refusal: wellfed_server_client::api::FeederRefusal::Busy,
                }),
            ),
        ];
        for entry in failed {
            assert!(settled(&entry).is_err(), "{entry:?}");
        }
    }
}
