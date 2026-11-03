//! The path router behind [`crate::api::router`]: the rules in [`core`], the
//! pins on disk in [`pins`], and the IO around them.

pub(crate) mod core;
pub(crate) mod io;
pub(crate) mod pins;
mod server;
mod shape;
