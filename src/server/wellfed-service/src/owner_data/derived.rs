//! The derived views, read from the store: a feeder's food view and a
//! pet's intake. This module gathers the typed rows (feeds with the food
//! each served, the pets each was split among, the beam's readings, the
//! missed meals, the calibration) and [`views`] does the arithmetic.
//!
//! A feed's food is [`Service::feeder_food_at`] at the time it dropped, so
//! a feed is counted in calories by the food it served, and its pets are
//! [`Service::feeder_pets`] over the view's span. Its weight is the
//! feeder's calibration alone.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use wellfed_api::{
    EventBody, EventKind, FOOD_VIEW_DAYS, FeederFood, FeederId, FoodPeriod, INTAKE_DAYS,
    INTAKE_DAYS_MAX, IntakeQuery, PetId, PetIntake, PortionWeight, Stats,
};

use super::views::{self, FeedRow, MissedRow, Served};
use crate::{Error, InternalError, Invalid, Result, Service};

impl Service {
    /// The feeder's food now (its newest change, even when the clock has
    /// stepped back before it), its newest refill, the portions since and
    /// (once calibrated) what the hopper has left, the feeder's rate, a
    /// beam-seen refill nobody recorded, and what went wrong over the last
    /// [`FOOD_VIEW_DAYS`] days.
    pub async fn feeder_food(&self, feeder: FeederId) -> Result<FeederFood> {
        self.row(feeder).await?;
        let store = &self.shared.store;
        let now = self.shared.clock.now();
        let week = views::before(now, FOOD_VIEW_DAYS);

        let current = store.current_food(feeder).await?;
        let last_refill = store.last_refill(feeder).await?;
        let weight = self.portion_weight(feeder).await?;
        let since = last_refill.as_ref().map_or(week, |refill| refill.at);
        let feeds = self.feed_rows(feeder, since.min(week), after(now)).await?;
        let readings = store.food_readings(feeder, since, after(now)).await?;
        // A meal is received after it was due, so every one due this week
        // was received this week too.
        let mut missed = self.missed(feeder, week, after(now)).await?;
        missed.retain(|missed| missed.due >= week);

        let rate = views::rate(now, FOOD_VIEW_DAYS, &feeds, weight);
        let grams_per_day = rate.and_then(|rate| rate.grams_per_day);
        let since_refill = last_refill
            .as_ref()
            .map(|refill| views::since_refill(now, refill, &feeds, weight, grams_per_day));
        let lately = feeds.iter().filter(|feed| feed.at >= week);
        Ok(FeederFood {
            current,
            last_refill,
            weight,
            since_refill,
            rate,
            refilled_unrecorded: views::refilled_unrecorded(&readings),
            anomalies: views::anomalies(lately, &missed),
        })
    }

    /// What the pet ate on each of the last `days` household days (today
    /// included), from every feeder that served it then, against its
    /// daily target; and what went wrong on those feeders those days.
    pub async fn pet_intake(&self, pet: PetId, query: IntakeQuery) -> Result<PetIntake> {
        let count = query.days.unwrap_or(INTAKE_DAYS);
        if !(1..=INTAKE_DAYS_MAX).contains(&count) {
            return Err(Invalid::IntakeDays.into());
        }
        let pet = self.pet(pet).await?;
        let now = self.shared.clock.now();
        let zone = self.mirror.zone();
        let tz = zone
            .as_ref()
            .map_or(TimeZone::UTC, |zone| zone.tz().clone());

        let dates = views::days(tz.to_datetime(now).date(), count);
        // The first day's midnight, or earlier should the zone not say:
        // every feed is placed on its own day, and one before the first is
        // left out.
        let start = dates
            .first()
            .and_then(|first| first.to_zoned(tz.clone()).ok())
            .map_or(Timestamp::MIN, |midnight| midnight.timestamp());
        let end = after(now);
        let mut sources = Vec::new();
        for feeder in self.links_feeders(pet.id, start, end).await? {
            let links = self.feeder_pets(feeder, start, end).await?;
            let weight = self.portion_weight(feeder).await?;
            let feeds = self.feed_rows(feeder, start, end).await?;
            let missed = self.missed(feeder, start, end).await?;
            sources.push((feeder, weight, links, feeds, missed));
        }

        let served: Vec<Served<'_>> = sources
            .iter()
            .map(|(feeder, weight, links, feeds, _)| Served {
                feeder: *feeder,
                weight: *weight,
                links,
                feeds,
            })
            .collect();
        let (feeders, days) = views::intake(pet.id, pet.daily_kcal, &dates, &tz, &served);
        let on_days = |at: Timestamp| at >= start;
        let feeds = sources
            .iter()
            .flat_map(|(_, _, _, feeds, _)| feeds)
            .filter(|feed| on_days(feed.at));
        let missed: Vec<MissedRow> = sources
            .iter()
            .flat_map(|(_, _, _, _, missed)| missed.iter().copied())
            .filter(|missed| on_days(missed.due))
            .collect();
        Ok(PetIntake {
            pet: pet.id,
            zone: zone.map_or_else(|| "UTC".to_owned(), |zone| zone.name().to_owned()),
            daily_kcal: pet.daily_kcal,
            feeders,
            days,
            anomalies: views::anomalies(feeds, &missed),
        })
    }

    /// How the feeder's portions are weighed: by the calibration in the
    /// portions statistics the server last read, when it has one.
    async fn portion_weight(&self, feeder: FeederId) -> Result<PortionWeight> {
        let stats = self.shared.store.latest_stats(feeder).await?;
        let calibration = stats.iter().find_map(|row| match &row.stats {
            Stats::Portions(portions) => portions.portion_mg,
            _ => None,
        });
        Ok(views::portion_weight(calibration))
    }

    /// The feeder's feeds received in `from..to`, each with the food it
    /// served when it dropped. Consecutive feeds mostly share a period, so
    /// one is asked for again only when a feed falls outside it.
    async fn feed_rows(
        &self,
        feeder: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<FeedRow>> {
        let feeds = self.shared.store.feeds(feeder, from, to).await?;

        let mut period: Option<FoodPeriod> = None;
        let mut rows = Vec::with_capacity(feeds.len());
        for feed in &feeds {
            let at = views::dropped(feed);
            let covers =
                |period: &FoodPeriod| period.from <= at && period.to.is_none_or(|to| at < to);
            if !period.as_ref().is_some_and(covers) {
                period = self.feeder_food_at(feeder, at).await?;
            }
            let food = period.as_ref().filter(|period| covers(period));
            rows.push(FeedRow::new(
                feeder,
                feed,
                food.map(|period| period.food.clone()),
            ));
        }
        Ok(rows)
    }

    /// The meals the feeder reported missed, received in `from..to`.
    async fn missed(
        &self,
        feeder: FeederId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<MissedRow>> {
        let rows = self
            .shared
            .store
            .events_received(feeder, EventKind::Missed, from, to)
            .await?;

        let mut missed = Vec::with_capacity(rows.len());
        for (epoch, row) in &rows {
            let event = wellfed_mirror::stored_event(*epoch, row)
                .ok_or(Error::Internal(InternalError::Undecodable(row.seq)))?;
            if let EventBody::Missed { plan, due, reason } = event.body {
                missed.push(MissedRow {
                    feeder,
                    plan,
                    due,
                    reason,
                });
            }
        }
        Ok(missed)
    }

    /// The feeders that served the pet at some time in `from..to`, each
    /// once, in the order their first link began.
    async fn links_feeders(
        &self,
        pet: PetId,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<FeederId>> {
        let links = self.shared.store.pet_links(pet, from, to).await?;

        let mut feeders: Vec<FeederId> = Vec::new();
        for link in links {
            if !feeders.contains(&link.feeder) {
                feeders.push(link.feeder);
            }
        }
        Ok(feeders)
    }
}

/// Just after `now`: the end of a window that holds what was received at
/// `now` itself, since the store's windows exclude their end.
fn after(now: Timestamp) -> Timestamp {
    now.checked_add(jiff::SignedDuration::from_millis(1))
        .unwrap_or(now)
}
