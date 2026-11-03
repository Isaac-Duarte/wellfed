//! `wellfed-api`'s bodies into the views Dart reads
//! ([`crate::api::server_views`]), and the server client's errors into
//! [`ServerError`]. Every conversion is total: a value the API adds fails
//! this crate's build until it has a view.

use jiff::Timestamp;
use wellfed_api::{
    Command, CommandOutcome, EventPage, FeedAnomaly, Feeder, FeederDetail, FeederEvent, FeederFood,
    FeederRefusal, Food, FoodLeft, FoodPeriod, Install, IntakeDay, IntakeFeeder, Pet, PetIntake,
    Refill, Role, SinceRefill, StatusSnapshot, StreamEvent, User,
};
use wellfed_model::{Event, Gap};
use wellfed_server_client::Error;

use crate::api::server::{
    ServerCommand, ServerCommandOutcome, ServerEvent, ServerInstall, ServerRefusal,
};
use crate::api::server_error::{ServerError, ServerErrorKind};
use crate::api::server_views::{
    ServerAccount, ServerAnomaly, ServerConnection, ServerEntry, ServerEventPage, ServerFeeder,
    ServerFeederDetail, ServerFeederFood, ServerFeederPortions, ServerFood, ServerFoodLeft,
    ServerFoodPeriod, ServerIntakeDay, ServerIntakeFeeder, ServerNextMeal, ServerPet,
    ServerPetIntake, ServerRefill, ServerRole, ServerSinceRefill, ServerSnapshot,
};
use crate::today::{Facts, event_entry, gap_entry};

/// Ids as Dart passes them back: the UUID strings the views gave.
pub(crate) mod ids {
    use uuid::Uuid;
    use wellfed_api::{FeederId, FoodId, PetId};

    use crate::api::server_error::{ServerError, ServerErrorKind};

    pub(crate) fn parse(id: &str) -> Result<Uuid, ServerError> {
        Uuid::parse_str(id).map_err(|error| {
            ServerError::new(ServerErrorKind::Invalid, format!("id {id:?}: {error}"))
        })
    }

    pub(crate) fn feeder(id: &str) -> Result<FeederId, ServerError> {
        parse(id).map(FeederId)
    }

    pub(crate) fn pet(id: &str) -> Result<PetId, ServerError> {
        parse(id).map(PetId)
    }

    pub(crate) fn food(id: &str) -> Result<FoodId, ServerError> {
        parse(id).map(FoodId)
    }
}

/// Dates as Dart passes them: `YYYY-MM-DD`.
pub(crate) mod date {
    use jiff::civil::Date;

    use crate::api::server_error::{ServerError, ServerErrorKind};

    pub(crate) fn parse(text: &str) -> Result<Date, ServerError> {
        text.parse().map_err(|error| {
            ServerError::new(ServerErrorKind::Invalid, format!("date {text:?}: {error}"))
        })
    }
}

fn ms(at: Timestamp) -> i64 {
    at.as_millisecond()
}

impl From<Error> for ServerError {
    fn from(error: Error) -> Self {
        use wellfed_api::ErrorCode;

        let retry_after_seconds = error
            .retry_after()
            .map(|wait| u32::try_from(wait.as_secs()).unwrap_or(u32::MAX));
        let kind = match &error {
            Error::Base(_) => ServerErrorKind::Address,
            Error::Http(http) if http.is_connect() => ServerErrorKind::Unreachable,
            Error::Http(_) => ServerErrorKind::Interrupted,
            Error::Problem { problem, .. } => match problem.code {
                ErrorCode::BadCredentials => ServerErrorKind::BadCredentials,
                ErrorCode::LoginLimited => ServerErrorKind::RateLimited,
                ErrorCode::Unauthenticated => ServerErrorKind::TokenRevoked,
                ErrorCode::AccountDisabled => ServerErrorKind::AccountDisabled,
                ErrorCode::Forbidden => ServerErrorKind::Forbidden,
                ErrorCode::NotFound => ServerErrorKind::NotFound,
                ErrorCode::Invalid => ServerErrorKind::Invalid,
                ErrorCode::FoodInUse => ServerErrorKind::FoodInUse,
                ErrorCode::FeederOffline | ErrorCode::Unreachable => ServerErrorKind::FeederOffline,
                // The rest a screen shows by the server's own title.
                _ => ServerErrorKind::Refused,
            },
            Error::Stale(_) | Error::StalePlans(_) => ServerErrorKind::Refused,
            Error::Build(_) | Error::Unexpected { .. } | Error::Decode(_) | Error::NoSession => {
                ServerErrorKind::Protocol
            }
        };

        let message = match &error {
            Error::Problem { problem, .. } => match &problem.detail {
                Some(detail) => format!("{}: {detail}", problem.title),
                None => problem.title.clone(),
            },
            other => other.to_string(),
        };
        Self {
            kind,
            message,
            retry_after_seconds,
        }
    }
}

impl From<Role> for ServerRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Owner => Self::Owner,
            Role::Member => Self::Member,
            Role::Viewer => Self::Viewer,
        }
    }
}

impl From<wellfed_api::Connection> for ServerConnection {
    fn from(connection: wellfed_api::Connection) -> Self {
        use wellfed_api::Connection;
        match connection {
            Connection::Offline => Self::Offline,
            Connection::Connecting => Self::Connecting,
            Connection::Online => Self::Online,
            Connection::Refused => Self::Refused,
        }
    }
}

impl From<User> for ServerAccount {
    fn from(user: User) -> Self {
        Self {
            id: user.id.to_string(),
            name: user.name,
            role: user.role.into(),
            expires_ms: user.expires.map(ms),
        }
    }
}

impl From<Feeder> for ServerFeeder {
    fn from(feeder: Feeder) -> Self {
        Self {
            id: feeder.id.to_string(),
            name: feeder.name,
            identity: feeder.identity.to_string(),
            role: feeder.role,
            address: feeder.address,
            connection: feeder.connection.into(),
            synced: feeder.synced,
            epoch: feeder.epoch,
            archived_ms: feeder.archived.map(ms),
        }
    }
}

impl From<FeederDetail> for ServerFeederDetail {
    fn from(detail: FeederDetail) -> Self {
        Self {
            feeder: detail.feeder.into(),
            status: detail.status.and_then(snapshot),
            next_meal: detail.next_meal.map(|meal| ServerNextMeal {
                at_ms: ms(meal.at),
                slot: meal.slot,
                portions: meal.portions,
            }),
        }
    }
}

/// The snapshot as the Home card reads it, from the status alone.
fn snapshot(snapshot: StatusSnapshot) -> Option<ServerSnapshot> {
    let facts = Facts {
        status: Some(snapshot.status),
        ..Facts::default()
    };
    Some(ServerSnapshot {
        at_ms: ms(snapshot.at),
        live: snapshot.live,
        card: facts.card()?,
    })
}

fn entry(event: &FeederEvent) -> ServerEntry {
    let received_ms = ms(event.received);
    let epoch = event.epoch;
    let event = Event {
        epoch,
        seq: event.seq,
        at: event.at,
        body: event.body,
    };
    ServerEntry {
        epoch,
        received_ms,
        entry: event_entry(&event),
    }
}

fn gap(gap: &wellfed_api::Gap) -> ServerEntry {
    let held = Gap {
        epoch: gap.epoch,
        from: gap.from,
        to: gap.to,
    };
    ServerEntry {
        epoch: gap.epoch,
        received_ms: ms(gap.received),
        entry: gap_entry(&held),
    }
}

/// The page's events and gaps in one list, oldest first, a gap where its
/// missing events would be.
pub(crate) fn event_page(page: &EventPage) -> ServerEventPage {
    let mut entries: Vec<ServerEntry> = page.gaps.iter().map(gap).collect();
    entries.extend(page.events.iter().map(entry));
    entries.sort_by_key(|entry| (entry.epoch, entry.entry.seq));

    ServerEventPage {
        epoch: page.epoch,
        entries,
        next: page.next,
    }
}

/// A stream event as Dart hears it; `None` for a sample, which a follow
/// never asks for.
pub(crate) fn stream_event(event: StreamEvent) -> Option<ServerEvent> {
    let event = match event {
        StreamEvent::Event { feeder, event } => ServerEvent::History {
            feeder: feeder.to_string(),
            entry: entry(&event),
        },
        StreamEvent::Gap { feeder, gap: held } => ServerEvent::History {
            feeder: feeder.to_string(),
            entry: gap(&held),
        },
        StreamEvent::Sample { .. } => return None,
        StreamEvent::Connection { feeder, connection } => ServerEvent::Connection {
            feeder: feeder.to_string(),
            connection: connection.into(),
        },
        StreamEvent::Command { command } => ServerEvent::Command {
            command: command.into(),
        },
        StreamEvent::Install { install, .. } => ServerEvent::Install {
            install: install.into(),
        },
        StreamEvent::Resync => ServerEvent::Resync,
    };
    Some(event)
}

impl From<Command> for ServerCommand {
    fn from(command: Command) -> Self {
        Self {
            key: command.key.to_string(),
            feeder: command.feeder.to_string(),
            kind: command.kind,
            state: command.state,
            outcome: command.outcome.map(ServerCommandOutcome::from),
            feed: command.feed,
            sent_ms: ms(command.sent),
            updated_ms: ms(command.updated),
        }
    }
}

impl From<CommandOutcome> for ServerCommandOutcome {
    fn from(outcome: CommandOutcome) -> Self {
        match outcome {
            CommandOutcome::Done => Self::Done,
            CommandOutcome::Settings { .. } => Self::Settings,
            CommandOutcome::Plans { .. } => Self::Plans,
            CommandOutcome::Refused { refusal } => Self::Refused {
                refusal: refusal.into(),
            },
            CommandOutcome::Invited { invite, seconds } => Self::Invited { invite, seconds },
            CommandOutcome::Feeding { feed } => Self::Feeding { feed },
            CommandOutcome::Fed {
                feed,
                requested,
                completed,
                outcome,
                reason,
            } => Self::Fed {
                feed,
                requested,
                completed,
                outcome,
                reason,
            },
            CommandOutcome::Unrecorded => Self::Unrecorded,
        }
    }
}

impl From<Install> for ServerInstall {
    fn from(install: Install) -> Self {
        Self {
            id: install.id.to_string(),
            feeder: install.feeder.to_string(),
            version: install.version,
            state: install.state,
            written: install.written,
            image_len: install.image_len,
            finished_ms: install.finished.map(ms),
        }
    }
}

impl From<FeederRefusal> for ServerRefusal {
    fn from(refusal: FeederRefusal) -> Self {
        match refusal {
            FeederRefusal::Forbidden => Self::Forbidden,
            FeederRefusal::Busy => Self::Busy,
            FeederRefusal::Stale => Self::Stale,
            FeederRefusal::Unavailable => Self::Unavailable,
            FeederRefusal::Invalid => Self::Invalid,
            FeederRefusal::NotWritable => Self::NotWritable,
            FeederRefusal::Full => Self::Full,
            FeederRefusal::NotFound => Self::NotFound,
            FeederRefusal::Muted => Self::Muted,
            FeederRefusal::Time => Self::Time,
            FeederRefusal::Fault => Self::Fault,
            FeederRefusal::Portions => Self::Portions,
            FeederRefusal::NoClock => Self::NoClock,
        }
    }
}

impl From<Food> for ServerFood {
    fn from(food: Food) -> Self {
        Self {
            id: food.id.to_string(),
            brand: food.brand,
            name: food.name,
            kcal_per_kg: food.kcal_per_kg,
            bag_grams: food.bag_grams,
        }
    }
}

impl From<FoodPeriod> for ServerFoodPeriod {
    fn from(period: FoodPeriod) -> Self {
        Self {
            food: period.food.into(),
            from_ms: ms(period.from),
            to_ms: period.to.map(ms),
        }
    }
}

impl From<Refill> for ServerRefill {
    fn from(refill: Refill) -> Self {
        Self {
            id: refill.id.to_string(),
            feeder: refill.feeder.to_string(),
            at_ms: ms(refill.at),
            grams: refill.grams,
            full_bag: refill.full_bag,
            food: refill.food.map(|food| food.to_string()),
            user: refill.user.to_string(),
        }
    }
}

impl From<FeederFood> for ServerFeederFood {
    fn from(food: FeederFood) -> Self {
        Self {
            current: food.current.map(ServerFoodPeriod::from),
            last_refill: food.last_refill.map(ServerRefill::from),
            weight: food.weight,
            since_refill: food.since_refill.map(ServerSinceRefill::from),
            rate: food.rate,
            refilled_unrecorded_ms: food.refilled_unrecorded.map(ms),
            anomalies: food
                .anomalies
                .into_iter()
                .map(ServerAnomaly::from)
                .collect(),
        }
    }
}

impl From<SinceRefill> for ServerSinceRefill {
    fn from(since: SinceRefill) -> Self {
        Self {
            portions: since.portions,
            uncertain_portions: since.uncertain_portions,
            left: since.left.map(ServerFoodLeft::from),
        }
    }
}

impl From<FoodLeft> for ServerFoodLeft {
    fn from(left: FoodLeft) -> Self {
        Self {
            grams_used: left.grams_used,
            grams_left: left.grams_left,
            grams_left_least: left.grams_left_least,
            empty_at_ms: left.empty_at.map(ms),
        }
    }
}

impl From<FeedAnomaly> for ServerAnomaly {
    fn from(anomaly: FeedAnomaly) -> Self {
        match anomaly {
            FeedAnomaly::Missed {
                feeder,
                plan,
                due,
                reason,
            } => Self::Missed {
                feeder: feeder.to_string(),
                plan,
                due_ms: ms(due),
                reason,
            },
            FeedAnomaly::Uncertain {
                feeder,
                feed,
                at,
                requested,
                completed,
            } => Self::Uncertain {
                feeder: feeder.to_string(),
                feed,
                at_ms: ms(at),
                requested,
                completed,
            },
            FeedAnomaly::Short {
                feeder,
                feed,
                at,
                requested,
                completed,
                outcome,
                reason,
            } => Self::Short {
                feeder: feeder.to_string(),
                feed,
                at_ms: ms(at),
                requested,
                completed,
                outcome,
                reason,
            },
        }
    }
}

impl From<Pet> for ServerPet {
    fn from(pet: Pet) -> Self {
        Self {
            id: pet.id.to_string(),
            name: pet.name,
            species: pet.species,
            birth_date: pet.birth_date.map(|date| date.to_string()),
            daily_kcal: pet.daily_kcal,
            feeders: pet.feeders.iter().map(ToString::to_string).collect(),
        }
    }
}

impl From<PetIntake> for ServerPetIntake {
    fn from(intake: PetIntake) -> Self {
        Self {
            pet: intake.pet.to_string(),
            zone: intake.zone,
            daily_kcal: intake.daily_kcal,
            feeders: intake.feeders.into_iter().map(Into::into).collect(),
            days: intake.days.into_iter().map(Into::into).collect(),
            anomalies: intake.anomalies.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<IntakeFeeder> for ServerIntakeFeeder {
    fn from(feeder: IntakeFeeder) -> Self {
        Self {
            feeder: feeder.feeder.to_string(),
            weight: feeder.weight,
            shared: feeder.shared,
        }
    }
}

impl From<IntakeDay> for ServerIntakeDay {
    fn from(day: IntakeDay) -> Self {
        Self {
            date: day.date.to_string(),
            weighed: day.weighed,
            target_percent: day.target_percent,
            estimate: day.estimate,
            unweighed: day
                .unweighed
                .into_iter()
                .map(|portions| ServerFeederPortions {
                    feeder: portions.feeder.to_string(),
                    portions: portions.portions,
                    uncertain_portions: portions.uncertain_portions,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_puts_each_gap_where_its_events_would_be() {
        let received = Timestamp::UNIX_EPOCH;
        let event = |seq| FeederEvent {
            epoch: 1,
            seq,
            at: None,
            received,
            body: wellfed_model::EventBody::Lost { count: 1 },
        };
        let page = EventPage {
            epoch: Some(1),
            events: vec![event(1), event(5)],
            gaps: vec![wellfed_api::Gap {
                epoch: 1,
                from: 2,
                to: 4,
                received,
            }],
            next: Some(5),
        };

        let page = event_page(&page);

        let seqs: Vec<u64> = page.entries.iter().map(|entry| entry.entry.seq).collect();
        assert_eq!(seqs, [1, 4, 5]);
        assert_eq!(page.next, Some(5));
    }

    #[test]
    fn a_dart_id_or_date_that_does_not_parse_is_invalid_not_a_panic() {
        assert_eq!(
            ids::feeder("kitchen").map_err(|error| error.kind),
            Err(ServerErrorKind::Invalid)
        );
        assert_eq!(
            date::parse("2020-13-01").map_err(|error| error.kind),
            Err(ServerErrorKind::Invalid)
        );
        assert!(date::parse("2020-05-01").is_ok());
    }
}
