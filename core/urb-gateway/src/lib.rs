//! The HTTP gateway (docs/rust-core.md, M8): the Go gateway (`main.go`,
//! `urb.go`) in Rust, with the same routes, headers and errors, its
//! pages rendered by `urb-render` and read by the core (whose reader
//! reads as reader.go does, its failures given the Go's codes and words
//! by [`routes::fail_of`]), and its state folder opened as the Go
//! gateway left it.
//!
//! - [`routes`] is `ServeHTTP` and what it calls: a request in, a
//!   response out, the reading handed in as a function.
//! - [`http`] is the part of Go's `net/http` the gateway's answers show:
//!   how a request's host, path and query are read, how `http.Error`
//!   and `http.Redirect` answer, and the HTTP/1.1 server itself, with
//!   Go's request parsing, timeouts and listen backlog, and a cap on
//!   the connections served at once.
//! - [`flags`] is Go's `flag` package, for the Go gateway's flags.
//! - [`state`] is the state folder: `comet.key`, `<ship>.db` and
//!   `<ship>.db.port`, as `main.go`'s `open` keeps them.
//! - [`golog`] is Go's `log`: a line with the local date and time.
#![cfg(not(target_family = "wasm"))]

pub mod flags;
pub mod golog;
pub mod http;
pub mod routes;
pub mod state;
