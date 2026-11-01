//! urb-fetch's protocol logic, on libmesa (docs/rust-core.md).
//!
//! M2 holds the nouns and the codecs: a safe noun wrapper over
//! mesa-sys ([`noun`]), the decoders of other ships' nouns with their
//! caps ([`codec`]), and the text forms of numbers and dates ([`text`]).
//! Each is a port of `wasm/mesa.mjs`, which is the specification: where
//! this crate would behave differently from the JS, this crate is wrong.

pub mod codec;
pub mod core;
pub mod engine;
pub mod forum;
pub mod js;
pub mod msg;
pub mod noun;
pub mod publish;
mod rt;
pub mod text;
pub mod url;

pub use core::{Core, Failure, Input, Output, Reply, Request};
pub use noun::{Int, Noun, NounRef, TypeError, Value, alive};

use std::fmt;

/// mesa.mjs's `UrbError`: what the reader and the decoders throw, with
/// the JS's message, word for word. `later` is set where the JS sets it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrbError {
    pub message: String,
    pub later: bool,
}

impl UrbError {
    pub fn new(message: impl Into<String>) -> Self {
        UrbError {
            message: message.into(),
            later: false,
        }
    }
}

impl fmt::Display for UrbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UrbError {}
