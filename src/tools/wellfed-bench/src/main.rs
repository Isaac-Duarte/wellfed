//! `wellfed-bench`: the bench server (`serve`), which holds the lease and
//! the relay board and runs the rig's sequences and the integration suite
//! one job at a time, and its clients: one command per sequence, `attach`,
//! `console` and `status`.
//! The sequences live in `wellfed-rig` and the suite in `wellfed-itest`;
//! this crate is the server that runs them and their command line.

#![forbid(unsafe_code)]

use std::process::ExitCode;

mod cli;
mod mcp;
mod note;
mod orphans;
mod tui;

fn main() -> ExitCode {
    cli::main()
}
