//! A count of the renderers' steps, for the fuzzers' assertion that
//! time is linear in a body's size. Every loop over a body's text
//! counts what it looks at. Without the `cost` feature it counts
//! nothing and costs nothing.

#[cfg(feature = "cost")]
use std::cell::Cell;

#[cfg(feature = "cost")]
thread_local! {
    static STEPS: Cell<u64> = const { Cell::new(0) };
}

/// counts n steps
#[inline(always)]
pub(crate) fn step(n: usize) {
    #[cfg(feature = "cost")]
    STEPS.with(|s| s.set(s.get() + n as u64));
    #[cfg(not(feature = "cost"))]
    let _ = n;
}

/// the steps counted on this thread since [`reset`]
#[cfg(feature = "cost")]
pub fn steps() -> u64 {
    STEPS.with(Cell::get)
}

/// starts the count again
#[cfg(feature = "cost")]
pub fn reset() {
    STEPS.with(|s| s.set(0));
}
