//! App parity: the app goes through the server first, so every device
//! operation it makes needs a server route. [`parity`] matches every
//! [`Op`] with no wildcard, so a new operation does not build until it is
//! given a route or a stated reason it has none.

use wellfed_model::Op;

/// An HTTP method, as the API document spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// Where a device operation is served through the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    /// This route, under `/api/v1`, in the API document's spelling.
    Route(Method, &'static str),
    /// No route, and why.
    Without(NoRoute),
}

/// Why a device operation has no server route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoRoute {
    /// The server keeps every feeder's clock and zone itself, from the
    /// household's zone; the zone is changed at `PATCH /household`.
    ClockKeptByServer,
    /// A step of an install, which the server drives as one operation of
    /// its own, owner-approved, from a verified release.
    Install,
    /// It needs a press at the feeder, so the owner runs it at the feeder
    /// with the CLI.
    AtTheFeeder,
}

/// The server route for a device operation, or why it has none.
pub const fn parity(op: &Op) -> Parity {
    use Method::{Delete, Get, Patch, Post, Put};
    use Parity::{Route, Without};

    match op {
        Op::Status => Route(Get, "/feeders/{id}"),
        Op::GetSettings => Route(Get, "/feeders/{id}/settings"),
        Op::PutSettings { .. } => Route(Patch, "/feeders/{id}/settings"),
        Op::PutWifi { .. } => Route(Put, "/feeders/{id}/wifi"),
        Op::SetLock { .. } => Route(Post, "/feeders/{id}/lock"),
        Op::ResetDesiccant => Route(Post, "/feeders/{id}/desiccant/reset"),
        Op::Play { .. } => Route(Post, "/feeders/{id}/play"),
        Op::CreateInvite { .. } => Route(Post, "/feeders/{id}/invites"),
        Op::ListClients => Route(Get, "/feeders/{id}/clients"),
        Op::RevokeClient { .. } => Route(Delete, "/feeders/{id}/clients/{key}"),
        Op::Feed { .. } => Route(Post, "/feeders/{id}/feed"),
        Op::Reboot => Route(Post, "/feeders/{id}/reboot"),
        Op::CancelFeed { .. } => Route(Post, "/feeders/{id}/feeds/{feed}/cancel"),
        Op::PutTime { .. } => Route(Patch, "/home"),
        Op::TimeSync(_) | Op::GetTime => Without(NoRoute::ClockKeptByServer),
        Op::OfferUpdate { .. } | Op::UpdateChunk { .. } | Op::UpdateState | Op::CancelUpdate => {
            Without(NoRoute::Install)
        }
        // The stock backup's read, a page per step on the long lane.
        Op::ReadBank1 { .. } => Route(Post, "/feeders/{id}/stock/read"),
        Op::EnrollUpdateKey { .. } | Op::UpdateKeyStatement { .. } => Without(NoRoute::AtTheFeeder),
        // The live debug log, tailed for the owners who follow it.
        Op::TailLog { .. } => Route(Get, "/feeders/{id}/log"),
        // The mirror reads these on its long lane and stores them beside
        // the history; the routes serve what it stored.
        Op::Stats { .. } => Route(Get, "/feeders/{id}/stats"),
        Op::FeedTrace { .. } => Route(Get, "/feeders/{id}/feeds/{feed}/trace"),
        Op::GetCalibration => Route(Get, "/feeders/{id}/calibration"),
        Op::PutCalibration { .. } => Route(Put, "/feeders/{id}/calibration"),
        Op::GetPlans => Route(Get, "/feeders/{id}/plans"),
        Op::PutPlans { .. } => Route(Put, "/feeders/{id}/plans"),
        Op::GetCall => Route(Get, "/feeders/{id}/call"),
        // The server takes the clip converted, and sends it a chunk at a
        // time on the long lane.
        Op::StartCall { .. } | Op::CallChunk { .. } => Route(Put, "/feeders/{id}/call"),
        Op::DeleteCall => Route(Delete, "/feeders/{id}/call"),
        Op::PlayCall => Route(Post, "/feeders/{id}/call/play"),
    }
}
