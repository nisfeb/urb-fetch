//! The wasm host (docs/rust-core.md, M6): the urb core built as
//! `mesa.wasm` for the JS shims in `shim/`, which keep mesa.mjs's,
//! pub.mjs's, furum.mjs's and forum.mjs's module interface.
//!
//! It exports a few plain functions. Text goes in through a buffer the
//! shim takes with `urb_alloc` and hands over, and every answer comes
//! back in one result buffer (`urb_out_ptr`, `urb_out_len`), valid until
//! the next call. A core's inputs and outputs are JSON in trace.mjs's
//! encoding ([`wire`]), so a trace's steps go in as they were recorded.
//! Entropy comes from the shim, through the import `host.entropy`.
//!
//! Each export returns 0 when its answer is in the buffer, and 1 when
//! the buffer holds the shim's mistake as text. Nothing another ship
//! sends reaches here except through the core, which never panics on
//! it; a panic would stop the module (wasm32-wasip1 aborts), and the
//! shim then refuses every later call.

pub mod wire;

#[cfg(feature = "render")]
pub use render_wasm;

use serde_json::{Value as J, json};
use std::cell::RefCell;
use urb::Core;

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static CORES: RefCell<Vec<Option<Core>>> = const { RefCell::new(Vec::new()) };
}

fn answer(r: Result<String, String>) -> i32 {
    let (code, text) = match r {
        Ok(t) => (0, t),
        Err(e) => (1, e),
    };
    OUT.with(|o| *o.borrow_mut() = text.into_bytes());
    code
}

/// n bytes for the shim to fill and hand to one call, which frees them
#[unsafe(no_mangle)]
pub extern "C" fn urb_alloc(n: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; n].into_boxed_slice()).cast()
}

/// bytes from [`urb_alloc`] that no call took
///
/// # Safety
/// `p` and `n` must be what [`urb_alloc`] gave, not freed before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn urb_free(p: *mut u8, n: usize) {
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, n)) });
}

#[unsafe(no_mangle)]
pub extern "C" fn urb_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn urb_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// the text the shim handed in, taken
unsafe fn taken(p: *mut u8, n: usize) -> Result<String, String> {
    let b = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, n)) };
    String::from_utf8(b.into_vec()).map_err(|_| "not UTF-8".to_string())
}

// (not in the unit tests, which make no core and run under wasmtime,
// where no host gives it)
#[cfg(all(target_arch = "wasm32", not(test)))]
#[link(wasm_import_module = "host")]
unsafe extern "C" {
    /// fill n bytes at p with entropy (the shim's `entropy(n)`)
    #[link_name = "entropy"]
    fn host_entropy(p: *mut u8, n: usize);
}

#[cfg(all(target_arch = "wasm32", not(test)))]
fn entropy(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    unsafe { host_entropy(b.as_mut_ptr(), n) };
    b
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn entropy(_: usize) -> Vec<u8> {
    unreachable!("the wasm host's entropy is the shim's")
}

/// a new core; its handle
#[unsafe(no_mangle)]
pub extern "C" fn urb_core_new() -> u32 {
    CORES.with(|c| {
        let mut c = c.borrow_mut();
        c.push(Some(Core::new(entropy)));
        (c.len() - 1) as u32
    })
}

/// let a core go, and its engine
#[unsafe(no_mangle)]
pub extern "C" fn urb_core_free(h: u32) {
    let core = CORES.with(|c| c.borrow_mut().get_mut(h as usize).and_then(Option::take));
    drop(core);
}

/// One input at now, as JSON (a trace step's `in`, with a call's `id`):
/// `{out, timers}`, the outputs it caused and the timers then set, each
/// `[kind, due]` in the order they were set.
///
/// # Safety
/// `p` and `n` must be what [`urb_alloc`] gave, filled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn urb_core_step(h: u32, now: f64, p: *mut u8, n: usize) -> i32 {
    let text = unsafe { taken(p, n) };
    answer(text.and_then(|t| step(h, now as u64, &t)))
}

fn step(h: u32, now: u64, text: &str) -> Result<String, String> {
    let i: J = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let input = wire::input(&i, 0)?;
    // the core is taken out while it steps, so nothing it calls finds
    // CORES borrowed
    let mut core = CORES
        .with(|c| c.borrow_mut().get_mut(h as usize).and_then(Option::take))
        .ok_or_else(|| format!("no core {h}"))?;
    let got = match input {
        Ok(input) => core
            .step(now, input)
            .map(|o| o.iter().map(wire::out).collect()),
        Err((id, f)) => Ok(vec![wire::failed(id, &f)]),
    };
    let timers: Vec<J> = core
        .timers()
        .into_iter()
        .map(|(k, d)| json!([k, d]))
        .collect();
    CORES.with(|c| c.borrow_mut()[h as usize] = Some(core));
    let out: Vec<J> = got?;
    Ok(json!({ "out": out, "timers": timers }).to_string())
}

/// what a keyfile is: `{ship, rank, rift, life}` (rift and life in
/// decimal, rift null when the keyfile doesn't say), or null
///
/// # Safety
/// As [`urb_core_step`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn urb_keyfile_info(p: *mut u8, n: usize) -> i32 {
    let text = unsafe { taken(p, n) };
    answer(text.map(|k| match urb::engine::keyfile_ship(&k) {
        None => "null".into(),
        Some((s, rift, life)) => {
            let rank =
                unsafe { mesa_sys::mesa_ship_rank(mesa_sys::mesa_ship { lo: s.0, hi: s.1 }) };
            json!({
                "ship": urb::engine::ship_text(s),
                "rank": rank,
                "rift": rift.map(|r| r.to_string()),
                "life": life.to_string(),
            })
            .to_string()
        }
    }))
}

/// a fake ship's keyfile, from its name; 1 with `not a ship: <name>`
///
/// # Safety
/// As [`urb_core_step`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn urb_fake_keyfile(p: *mut u8, n: usize) -> i32 {
    let text = unsafe { taken(p, n) };
    answer(text.and_then(|s| match urb::engine::ship_parse(&s) {
        Some(ship) => Ok(urb::engine::fake_keyfile(ship)),
        None => Err(format!("not a ship: {s}")),
    }))
}

/// the @p of a ship number, as its two 64-bit halves
#[unsafe(no_mangle)]
pub extern "C" fn urb_ship_name(lo: u64, hi: u64) -> i32 {
    answer(Ok(urb::engine::ship_text((lo, hi))))
}

/// `+sham` of a value (trace.mjs's encoding), in decimal
///
/// # Safety
/// As [`urb_core_step`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn urb_sham(p: *mut u8, n: usize) -> i32 {
    let text = unsafe { taken(p, n) };
    answer(text.and_then(|t| hash(&t, |v| urb::noun::sham(v).map(|h| h.to_string()))))
}

/// `+mug` of a value (trace.mjs's encoding)
///
/// # Safety
/// As [`urb_core_step`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn urb_mug(p: *mut u8, n: usize) -> i32 {
    let text = unsafe { taken(p, n) };
    answer(text.and_then(|t| hash(&t, |v| urb::noun::mug(v).map(|h| h.to_string()))))
}

/// an error as text: `<class>: <message>`, for the shim to throw
fn hash(
    text: &str,
    f: impl Fn(&urb::Value<'_>) -> Result<String, urb::TypeError>,
) -> Result<String, String> {
    let j: J = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let v = wire::value(&j).map_err(|e| e.log_text())?;
    f(&v).map_err(|e| format!("TypeError: {}", e.0))
}
