//! Adopted feeders: the list, adoption, status, settings, plans, a skip of
//! one of today's meals, the panel lock, the history and the samples.

use reqwest::header;
use reqwest::{Method, StatusCode};
use uuid::Uuid;
use wellfed_api::{
    AdoptFeeder, EventPage, EventQuery, Feeder, FeederDetail, FeederId, FeederSettingsPatch,
    Generation, LockRequest, MealRequest, Plans, PlansChange, SamplePage, SampleQuery, Settings,
};

use crate::{Client, Error, decode, decode_page, expect_status, stale_or};

/// What a lock request asks of the feeder's panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// The panel's buttons do nothing.
    Locked,
    Unlocked,
}

impl Client {
    /// The adopted feeders, with the server's connection to each.
    pub async fn feeders(&self) -> Result<Vec<Feeder>, Error> {
        let response = self.api(Method::GET, "/feeders").send().await?;
        decode(response, StatusCode::OK).await
    }

    /// Adopt a feeder by its invite for the server. Owners only.
    pub async fn adopt(&self, request: &AdoptFeeder) -> Result<Feeder, Error> {
        let response = self
            .api(Method::POST, "/feeders")
            .json(request)
            .send()
            .await?;
        decode(response, StatusCode::CREATED).await
    }

    /// One feeder and its status.
    pub async fn feeder(&self, id: FeederId) -> Result<FeederDetail, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}"))
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// The feeder's settings, read from it. Their `generation` is the base
    /// an edit names.
    pub async fn settings(&self, id: FeederId) -> Result<Settings, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/settings"))
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// Change settings, if they are still at `base`. A retry carries the
    /// same `key`; settings that moved on come back as [`Error::Stale`].
    pub async fn put_settings(
        &self,
        id: FeederId,
        base: Generation,
        patch: &FeederSettingsPatch,
        key: Uuid,
    ) -> Result<Settings, Error> {
        let response = self
            .keyed(Method::PATCH, &format!("/feeders/{id}/settings"), key)
            .header(header::IF_MATCH, format!("\"{base}\""))
            .json(patch)
            .send()
            .await?;
        stale_or(response, Error::Stale).await
    }

    /// The feeder's plans, read from it. Their `generation` is the base a
    /// change names.
    pub async fn plans(&self, id: FeederId) -> Result<Plans, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/plans"))
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// Replace every plan slot, if the plans are still at `base`. A retry
    /// carries the same `key`; plans that moved on come back as
    /// [`Error::StalePlans`].
    pub async fn put_plans(
        &self,
        id: FeederId,
        base: Generation,
        change: &PlansChange,
        key: Uuid,
    ) -> Result<Plans, Error> {
        let response = self
            .keyed(Method::PUT, &format!("/feeders/{id}/plans"), key)
            .header(header::IF_MATCH, format!("\"{base}\""))
            .json(change)
            .send()
            .await?;
        stale_or(response, Error::StalePlans).await
    }

    /// Skip one of today's meals, named by its plan's slot and its instant.
    /// A retry carries the same `key`.
    pub async fn skip_meal(&self, id: FeederId, meal: MealRequest, key: Uuid) -> Result<(), Error> {
        self.meal(id, "skip", meal, key).await
    }

    /// Undo a meal's skip while it is still to come. A retry carries the
    /// same `key`.
    pub async fn unskip_meal(
        &self,
        id: FeederId,
        meal: MealRequest,
        key: Uuid,
    ) -> Result<(), Error> {
        self.meal(id, "unskip", meal, key).await
    }

    async fn meal(
        &self,
        id: FeederId,
        verb: &str,
        meal: MealRequest,
        key: Uuid,
    ) -> Result<(), Error> {
        let response = self
            .keyed(Method::POST, &format!("/feeders/{id}/meals/{verb}"), key)
            .json(&meal)
            .send()
            .await?;
        expect_status(response, StatusCode::NO_CONTENT).await?;
        Ok(())
    }

    /// Lock or unlock the feeder's panel. A retry carries the same `key`.
    pub async fn set_lock(&self, id: FeederId, lock: Lock, key: Uuid) -> Result<(), Error> {
        let response = self
            .keyed(Method::POST, &format!("/feeders/{id}/lock"), key)
            .json(&LockRequest {
                locked: lock == Lock::Locked,
            })
            .send()
            .await?;
        expect_status(response, StatusCode::NO_CONTENT).await?;
        Ok(())
    }

    /// Record that the feeder's desiccant was replaced today. A retry
    /// carries the same `key`.
    pub async fn reset_desiccant(&self, id: FeederId, key: Uuid) -> Result<(), Error> {
        let response = self
            .keyed(Method::POST, &format!("/feeders/{id}/desiccant/reset"), key)
            .send()
            .await?;
        expect_status(response, StatusCode::NO_CONTENT).await?;
        Ok(())
    }

    /// A page of the feeder's history as the server holds it. An event
    /// this client cannot read (a kind a newer feeder or server added) is
    /// left out; the page's `next` still moves past it.
    pub async fn events(&self, id: FeederId, query: &EventQuery) -> Result<EventPage, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/events"))
            .query(query)
            .send()
            .await?;
        decode_page(response, StatusCode::OK, "events").await
    }

    /// A page of one metric's samples, raw or by the hour.
    pub async fn samples(&self, id: FeederId, query: &SampleQuery) -> Result<SamplePage, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/samples"))
            .query(query)
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }
}
