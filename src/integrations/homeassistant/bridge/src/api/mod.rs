//! The values Home Assistant reads, as typed views of `wellfed-model`'s.
//!
//! Each is a plain Rust type the session builds and the tests check; with
//! the `python` feature each is also a Python class, and the module's stub
//! (`wellfed_homeassistant.pyi`) is generated from them, so the
//! integration's Python is type-checked against these files rather than
//! reading keys out of JSON. Closed sets are Python enums whose `key` is
//! the snake_case name Home Assistant uses for an entity's state and its
//! translation key.
//!
//! A view keeps only what an entity or an event shows. Anything the
//! feeder checks stays in Rust: these carry facts out, never a request in.

#[macro_use]
mod enums;
mod alerts;
mod session;
mod state;
mod views;

pub use alerts::{Alert, AlertKind, AlertLevel, Alerts};
pub use enums::{
    AccountRole, By, EventKind, FeedOutcome, FeedReason, FoodLevel, NotSent, Readiness,
    RefusalKind, Role, ServerProblem, Skew, Tune, UpdateKind, WriteResult,
};
pub use session::{Pending, Update, Written};
pub use state::{Capabilities, NextMeal, Plan, PortionsToday, Settings, State};
pub use views::{Event, Feed, FeedFault, Status};
