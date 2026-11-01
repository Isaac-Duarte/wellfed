//! The bench server behind the `wellfed-bench` binary. It is a library only
//! so that the terminal and MCP tests in `tests/` can run a real server
//! next to the binary they drive; nothing else links it.

#![forbid(unsafe_code)]

pub mod server;
