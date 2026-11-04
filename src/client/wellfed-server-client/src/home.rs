//! The home: its id, name, units, time format and zone.

use reqwest::{Method, StatusCode};
use wellfed_api::{Home, HomePatch};

use crate::{Client, Error, decode};

impl Client {
    pub async fn home(&self) -> Result<Home, Error> {
        let response = self.api(Method::GET, "/home").send().await?;
        decode(response, StatusCode::OK).await
    }

    /// Change the home; a new zone goes to every feeder. Owners only.
    pub async fn update_home(&self, patch: &HomePatch) -> Result<Home, Error> {
        let response = self.api(Method::PATCH, "/home").json(patch).send().await?;
        decode(response, StatusCode::OK).await
    }
}
