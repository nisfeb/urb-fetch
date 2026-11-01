//! `urb`, the native command line (docs/rust-core.md, M7): the Node
//! command lines in wasm/, on the native host, with their flags, output
//! and exit codes.
//!
//!   urb read  [options] [--tries N] urb://~ship/page      (host.mjs)
//!   urb serve [options] [--put NAME=FILE]... [--remove NAME]...
//!                                                         (pub-cli.mjs)
//!   urb furum [options] ~host board [...]                 (furum-cli.mjs)
//!   urb forum [options] urb://~host/forum/board[/ID] [...] (forum-cli.mjs)
//!
//! The options are node.mjs's: --state DIR, --relay URL, --moon KEYFILE,
//! --new-moon, --fake ~SHIP, --trace. The state folder is node.mjs's
//! too (comet.key, rows, port, pages.json, by default under
//! ~/.local/state/urb-fetch-wasm), so a folder moves between the two.

#[cfg(not(target_family = "wasm"))]
mod args;
#[cfg(not(target_family = "wasm"))]
mod cli;

#[cfg(not(target_family = "wasm"))]
fn main() {
    std::process::exit(cli::main(std::env::args().skip(1).collect()));
}

#[cfg(target_family = "wasm")]
fn main() {
    eprintln!("urb: the native command line does not run as wasm");
    std::process::exit(1);
}
