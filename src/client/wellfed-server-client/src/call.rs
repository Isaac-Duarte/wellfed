//! A feeder's portion calibration and its meal call, as the server serves
//! them.

use reqwest::header::{self, HeaderValue};
use reqwest::{Method, StatusCode};
use uuid::Uuid;
use wellfed_api::{Calibration, CalibrationView, CallView, FeederId, Generation};

use crate::feeders::IDEMPOTENCY_KEY;
use crate::{Client, Error, decode, expect_status, is_problem};

impl Client {
    /// The feeder's portion calibration, read from it. Its `generation`
    /// is the base a change names.
    pub async fn calibration(&self, id: FeederId) -> Result<CalibrationView, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/calibration"))
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// Store `calibration`, if the feeder's is still at `base`. A retry
    /// carries the same `key`; a calibration that moved on comes back as
    /// [`Error::StaleCalibration`].
    pub async fn put_calibration(
        &self,
        id: FeederId,
        base: Generation,
        calibration: &Calibration,
        key: Uuid,
    ) -> Result<CalibrationView, Error> {
        let response = self
            .api(Method::PUT, &format!("/feeders/{id}/calibration"))
            .header(header::IF_MATCH, format!("\"{base}\""))
            .header(IDEMPOTENCY_KEY, key.to_string())
            .json(calibration)
            .send()
            .await?;

        // A stale change is answered with the current calibration.
        if response.status() == StatusCode::PRECONDITION_FAILED && !is_problem(&response) {
            let current = decode(response, StatusCode::PRECONDITION_FAILED).await?;
            return Err(Error::StaleCalibration(Box::new(current)));
        }
        decode(response, StatusCode::OK).await
    }

    /// The feeder's meal call: the stored clip, or an upload under way.
    pub async fn call(&self, id: FeederId) -> Result<CallView, Error> {
        let response = self
            .api(Method::GET, &format!("/feeders/{id}/call"))
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// Upload `stream`, a clip already in the feeder's format
    /// (`wellfed_client::call::Clip::stream`), as the meal call. The
    /// answer comes once the feeder stores it. A retry after an unknown
    /// outcome carries the same `key` and the same stream, and sends only
    /// what the feeder lacks.
    pub async fn upload_call(
        &self,
        id: FeederId,
        stream: Vec<u8>,
        key: Uuid,
    ) -> Result<CallView, Error> {
        let response = self
            .api(Method::PUT, &format!("/feeders/{id}/call"))
            .header(IDEMPOTENCY_KEY, key.to_string())
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            )
            .body(stream)
            .send()
            .await?;
        decode(response, StatusCode::OK).await
    }

    /// Erase the meal call. A retry carries the same `key`.
    pub async fn delete_call(&self, id: FeederId, key: Uuid) -> Result<(), Error> {
        let response = self
            .api(Method::DELETE, &format!("/feeders/{id}/call"))
            .header(IDEMPOTENCY_KEY, key.to_string())
            .send()
            .await?;
        expect_status(response, StatusCode::NO_CONTENT).await?;
        Ok(())
    }

    /// Play the stored meal call once on the feeder. A retry carries the
    /// same `key`.
    pub async fn play_call(&self, id: FeederId, key: Uuid) -> Result<(), Error> {
        let response = self
            .api(Method::POST, &format!("/feeders/{id}/call/play"))
            .header(IDEMPOTENCY_KEY, key.to_string())
            .send()
            .await?;
        expect_status(response, StatusCode::NO_CONTENT).await?;
        Ok(())
    }
}
