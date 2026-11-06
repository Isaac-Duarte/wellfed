//! The path router behind [`crate::api::router`]: the rules in [`core`], the
//! pins on disk in [`pins`], the IO around them, and adopting a feeder this
//! phone paired before the server ([`adopt`]).

mod adopt;
pub(crate) mod core;
pub(crate) mod io;
pub(crate) mod pins;
mod server;
mod shape;
