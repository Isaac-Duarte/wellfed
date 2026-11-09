//! The audit log: who did what through the server.

use reqwest::{Method, StatusCode};
use wellfed_api::{AuditPage, PageQuery};

use crate::{Client, Error, decode_page};

impl Client {
    /// A page of the audit log, oldest first: the caller's own entries, or
    /// everyone's for an owner. The next page passes the page's `next` as
    /// `after`. An entry a newer server added, which this client cannot
    /// read, is left out.
    pub async fn audit(&self, query: &PageQuery) -> Result<AuditPage, Error> {
        let response = self.api(Method::GET, "/audit").query(query).send().await?;
        decode_page(response, StatusCode::OK, "items").await
    }
}
