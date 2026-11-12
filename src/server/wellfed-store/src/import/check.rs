//! Every row of an archive read by the store's own readers before it goes
//! in. The database takes any text and any integer, but its readers refuse
//! a code they do not know, a number wider than the value it stands for and
//! columns that contradict each other, and some of them run at every start
//! (the home, the feeders, the projections) or every login (an account's
//! role): an archive with such a row, hand-edited or from a build with
//! another vocabulary, would commit a server that cannot start. Rows go in
//! exactly as exported, so the conversion each reader runs on a stored row
//! runs here on the archive's, and a row it refuses refuses the import,
//! which rolls it all back.

use wellfed_api::{Food, Install, MetricKind, PhoneKey};
use wellfed_formats::export::Row;

use super::Decoders;
use crate::commands::{self, CommandRow};
use crate::phones;
use crate::projections::{Feed, FeedRow, Projected, read};
use crate::samples::{self, rollups};
use crate::{
    Error, Result, audit, diagnostics, feeders, foods, history, homes, installs, pets, refills,
    users,
};

/// Refuse `row` if a store reader, or a page decoding what it keeps by
/// `decoders`, would refuse it once stored.
pub(super) fn check(row: &Row, decoders: Decoders) -> Result<()> {
    readable(row, decoders).map_err(|error| match error {
        Error::Corrupt(what) => Error::Unreadable(what),
        error => error,
    })
}

fn readable(row: &Row, decoders: Decoders) -> Result<()> {
    match row {
        Row::Users(user) => {
            users::user(user.id, user.name.clone(), &user.role, user.expires_ms)?;
        }
        Row::Homes(home) => {
            homes::home(
                home.id,
                home.name.clone(),
                &home.units,
                &home.time_format,
                home.zone.clone(),
            )?;
        }
        Row::Feeders(feeder) => {
            // As the import stores it: archived, with no key.
            feeders::feeder_row(feeders::Selected {
                id: feeder.id,
                home_id: feeder.home_id,
                kind: feeder.model.kind().name().to_owned(),
                model: feeder.model.name().to_owned(),
                identity: feeder.identity.clone(),
                name: feeder.name.clone(),
                role: feeder.role.clone(),
                address: feeder.address.clone(),
                key_nonce: vec![0; 12],
                key_sealed: Vec::new(),
                synced: feeder.synced,
                epoch: feeder.epoch,
                archived_ms: None,
            })?;
        }
        Row::FeederState(state) => {
            feeders::state(
                (state.status.clone(), state.status_ms),
                (state.settings.clone(), state.settings_ms),
                (state.plans.clone(), state.plans_ms),
            )?;
        }
        Row::PhoneKeys(phone) => {
            PhoneKey::try_from(phones::Raw {
                feeder_id: phone.feeder_id,
                key: phone.key.clone(),
                user_id: phone.user_id,
                token_id: phone.token_id,
                role: phone.role.clone(),
                invited_ms: phone.invited_ms,
                invite_until_ms: phone.invite_until_ms,
                revoke: phone.revoke.clone(),
                revoke_ms: phone.revoke_ms,
                attempt: phone.attempt.clone(),
            })?;
        }
        Row::KeptClients(kept) => {
            phones::kept_key(kept.key.clone())?;
        }
        Row::Invitations(row) => {
            phones::stored_invitation(
                row.epoch,
                row.seq,
                row.invitee.clone(),
                row.inviter.clone(),
                row.received_ms,
            )?;
        }
        // Read only by key and prefix, which the schema checks.
        Row::DroppedPhones(_) => {}
        Row::Events(event) => {
            history::event_epoch(event.epoch)?;
            let read = history::event(
                event.seq,
                event.at_s,
                event.received_ms,
                &event.kind,
                Vec::new(),
            )?;
            // A body of a kind this build does not know is kept and skipped;
            // one it knows is decoded by every page that lists it.
            if read.kind.is_some() && !(decoders.event)(&event.body) {
                return Err(Error::Corrupt("event body"));
            }
        }
        Row::Gaps(gap) => {
            let epoch = history::event_epoch(gap.epoch)?;
            history::gap(epoch, gap.from_seq, gap.to_seq, gap.received_ms)?;
        }
        Row::Samples(sample) => {
            samples::times(sample.received_ms, sample.uptime_ms)?;
            // A metric this build does not know is never asked for.
            if MetricKind::from_code(&sample.metric).is_some() && !(decoders.sample)(&sample.value)
            {
                return Err(Error::Corrupt("sample value"));
            }
        }
        Row::SampleRollups(rollup) => {
            rollups::hour(
                rollup.hour_ms,
                rollup.count,
                rollup.min,
                rollup.max,
                rollup.sum,
            )?;
        }
        // Marks a start compares with its own, and rebuilds from when they
        // differ.
        Row::SampleMarks(_) | Row::ProjectionVersion(_) => {}
        Row::Feeds(feed) => {
            Projected::<Feed>::try_from(FeedRow {
                epoch: feed.epoch,
                feed: feed.feed,
                seq: feed.seq,
                source: feed.source.clone(),
                client: feed.client.clone(),
                requested: feed.requested,
                completed: feed.completed,
                outcome: feed.outcome.clone(),
                reason: feed.reason.clone(),
                at_s: feed.at_s,
                received_ms: feed.received_ms,
            })?;
        }
        Row::LockChanges(change) => {
            let value = read::lock_change(change.locked, &change.source, change.client.clone())?;
            read::projected(
                change.epoch,
                change.seq,
                change.at_s,
                change.received_ms,
                value,
            )?;
        }
        Row::Boots(boot) => {
            let value = read::boot(boot.bank, boot.reset_flags)?;
            read::projected(boot.epoch, boot.seq, boot.at_s, boot.received_ms, value)?;
        }
        Row::ClockSyncs(sync) => {
            let value = read::clock_sync(&sync.source, sync.client.clone(), sync.jump_ms)?;
            read::projected(sync.epoch, sync.seq, sync.at_s, sync.received_ms, value)?;
        }
        Row::BlockedChanges(change) => {
            let value = change.reason.as_deref().map(read::blocked);
            read::projected(
                change.epoch,
                change.seq,
                change.at_s,
                change.received_ms,
                value,
            )?;
        }
        Row::FeedTraces(trace) => {
            diagnostics::trace(
                (trace.epoch, trace.feed, trace.fetched_ms),
                trace.summary.clone(),
                trace.stored,
                trace.body.clone(),
                trace.refusal.clone(),
            )?;
        }
        Row::FeederStats(stat) => {
            diagnostics::stats_row(stat.received_ms, stat.boot_count, &stat.stats)?;
        }
        Row::FoodReadings(reading) => {
            diagnostics::food_reading(
                (reading.received_ms, reading.boot_count),
                reading.present,
                (
                    reading.ran_low,
                    reading.refilled,
                    reading.feeds_since_refill,
                ),
            )?;
        }
        Row::Commands(command) => {
            CommandRow::try_from(commands::Raw {
                seq: command.seq,
                key: command.key.clone(),
                user_id: command.user_id,
                token_id: command.token_id,
                feeder_id: command.feeder_id,
                kind: command.kind.clone(),
                body_hash: command.body_hash.clone(),
                request_id: command.request_id.clone(),
                boot_count: command.boot_count,
                history_epoch: command.history_epoch,
                history_seq: command.history_seq,
                state: command.state.clone(),
                outcome: command.outcome.clone(),
                feed: command.feed,
                sent_ms: command.sent_ms,
                updated_ms: command.updated_ms,
            })?;
        }
        Row::Audit(entry) => {
            // The journal entry it names went in, and was read, before it.
            audit::entry(entry.command_seq, entry.action.as_deref())?;
            audit::stamp(entry.seq, entry.at_ms)?;
        }
        Row::Installs(install) => {
            Install::try_from(installs::Raw {
                id: install.id,
                feeder_id: install.feeder_id,
                target: install.target.clone(),
                release_id: install.release_id.clone(),
                version: install.version,
                user_id: install.user_id,
                started_ms: install.started_ms,
                state: install.state.clone(),
                written: install.written,
                image_len: install.image_len,
                failure: install.failure.clone(),
                finished_ms: install.finished_ms,
            })?;
        }
        Row::Pets(pet) => {
            pets::pet(
                pet.id,
                pet.name.clone(),
                &pet.species,
                pet.birth_date.clone(),
                pet.daily_kcal,
                Vec::new(),
            )?;
        }
        Row::PetWeights(weight) => {
            pets::weight(weight.id, weight.at_ms, weight.grams)?;
        }
        Row::PetFeeders(link) => {
            pets::link_span(link.from_ms, link.to_ms)?;
        }
        Row::Foods(food) => {
            Food::try_from(foods::Raw {
                id: food.id,
                brand: food.brand.clone(),
                name: food.name.clone(),
                kcal_per_kg: food.kcal_per_kg,
                bag_grams: food.bag_grams,
            })?;
        }
        Row::FeederFoods(food) => {
            foods::period_start(food.seq, food.from_ms)?;
        }
        Row::Refills(refill) => {
            refills::Raw {
                seq: refill.seq,
                id: refill.id,
                feeder_id: refill.feeder_id,
                at_ms: refill.at_ms,
                grams: refill.grams,
                full_bag: refill.full_bag,
                food_id: refill.food_id,
                user_id: refill.user_id,
            }
            .refill()?;
        }
    }
    Ok(())
}
