//! libmesa's C API, as bindgen reads it from `include/mesa/*.h`.
//!
//! No signature here is written by hand, and there is no logic: the
//! safe wrappers live in the `urb` crate. libmesa itself is built by
//! build.rs, natively or for wasm32-wasip1.

#![no_std]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
