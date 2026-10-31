//! The order of a step's outputs, as the JS makes it.
//!
//! mesa.mjs runs on promises: a datagram goes out two jobs after the
//! commit before it, a read's result some jobs after the answer that
//! settles it, and the outputs of one input land in the order the JS's
//! job queue runs them. A trace records that order, so the core keeps
//! the same queue: promises that settle and run their reactions as jobs,
//! in order, as ECMAScript's do, and async functions that await them.
//! Each piece of mesa.mjs is ported with its awaits where the JS has
//! them, and the order of every output follows, with no reckoning of
//! depths by hand.
//!
//! - [`then`] is PerformPromiseThen: a reaction runs as a job once its
//!   promise settles.
//! - [`resolve`] with a promise adopts it as the JS does, through
//!   NewPromiseResolveThenableJob: two jobs later.
//! - [`spawn`] runs an async function's body at once, up to its first
//!   [`Step::Await`], which resumes as a reaction (one job after the
//!   promise awaited settles, as V8's await does since 7.2).
//!
//! The queue drains at the end of each input, as Node's microtasks do
//! before the next macrotask.

use crate::core::{Val, World};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// a promise's outcome: fulfilled or rejected
pub(crate) type Settled = Result<Val, Val>;

/// a reaction: what a `then` runs, and what its promise settles with
pub(crate) type Handler = Box<dyn FnOnce(&World, Settled) -> Settled>;

enum State {
    Pending(Vec<(Handler, Promise)>),
    Done(Settled),
}

struct Slot {
    state: State,
    // a resolve function was called (it may still be adopting another)
    resolved: bool,
}

#[derive(Clone)]
pub(crate) struct Promise(Rc<RefCell<Slot>>);

impl Promise {
    pub fn new() -> Promise {
        Promise(Rc::new(RefCell::new(Slot {
            state: State::Pending(Vec::new()),
            resolved: false,
        })))
    }

    /// Promise.resolve(v): v itself if it is a promise
    pub fn of(v: Val) -> Promise {
        match v {
            Val::Promise(p) => p,
            v => Promise(Rc::new(RefCell::new(Slot {
                state: State::Done(Ok(v)),
                resolved: true,
            }))),
        }
    }
}

enum Job {
    React(Handler, Settled, Promise),
    Adopt(Promise, Promise),
}

#[derive(Default)]
pub(crate) struct Jobs(RefCell<VecDeque<Job>>);

impl Jobs {
    fn push(&self, j: Job) {
        self.0.borrow_mut().push_back(j);
    }

    /// run every job, and every job they queue, in order
    pub fn drain(&self, w: &World) {
        loop {
            let Some(job) = self.0.borrow_mut().pop_front() else {
                return;
            };
            match job {
                Job::React(h, v, d) => {
                    let r = h(w, v);
                    resolve(w, &d, r);
                }
                Job::Adopt(p, x) => {
                    then(
                        w,
                        &x,
                        // fresh resolving functions: p was marked resolved
                        // when it was handed x
                        Box::new(move |w, r| {
                            fulfil(w, &p, r);
                            Ok(Val::Undef)
                        }),
                    );
                }
            }
        }
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

/// p's resolve (with Ok) or reject (with Err) function: once only. A
/// promise it is resolved with is adopted, two jobs on.
pub(crate) fn resolve(w: &World, p: &Promise, v: Settled) {
    {
        let mut s = p.0.borrow_mut();
        if s.resolved {
            return;
        }
        s.resolved = true;
    }
    fulfil(w, p, v);
}

/// resolve, past the once-only check
fn fulfil(w: &World, p: &Promise, v: Settled) {
    match v {
        Ok(Val::Promise(x)) => w.jobs.push(Job::Adopt(p.clone(), x)),
        v => settle(w, p, v),
    }
}

fn settle(w: &World, p: &Promise, v: Settled) {
    let old = std::mem::replace(&mut p.0.borrow_mut().state, State::Done(v.clone()));
    if let State::Pending(reactions) = old {
        for (h, d) in reactions {
            w.jobs.push(Job::React(h, v.clone(), d));
        }
    }
}

/// p.then(h): h runs as a job once p settles, and the promise returned
/// settles with what h gives
pub(crate) fn then(w: &World, p: &Promise, h: Handler) -> Promise {
    let d = Promise::new();
    let mut s = p.0.borrow_mut();
    match &mut s.state {
        State::Pending(list) => list.push((h, d.clone())),
        State::Done(v) => {
            let v = v.clone();
            drop(s);
            w.jobs.push(Job::React(h, v, d.clone()));
        }
    }
    d
}

/// p.then(f): f on a fulfilment; a rejection passes through
pub(crate) fn on_ok(
    w: &World,
    p: &Promise,
    f: impl FnOnce(&World, Val) -> Settled + 'static,
) -> Promise {
    then(
        w,
        p,
        Box::new(move |w, r| match r {
            Ok(v) => f(w, v),
            Err(e) => Err(e),
        }),
    )
}

/// p.catch(f): f on a rejection; a fulfilment passes through
pub(crate) fn on_err(
    w: &World,
    p: &Promise,
    f: impl FnOnce(&World, Val) -> Settled + 'static,
) -> Promise {
    then(
        w,
        p,
        Box::new(move |w, r| match r {
            Ok(v) => Ok(v),
            Err(e) => f(w, e),
        }),
    )
}

/// p.finally(f), as the JS runs it: f, then what p settled with, a job
/// after Promise.resolve(f()) settles
pub(crate) fn finally(w: &World, p: &Promise, f: impl FnOnce(&World) + 'static) -> Promise {
    then(
        w,
        p,
        Box::new(move |w, r| {
            f(w);
            let after = then(w, &Promise::of(Val::Undef), Box::new(move |_, _| r));
            Ok(Val::Promise(after))
        }),
    )
}

/// where an async function is: done, failed, or awaiting a value
pub(crate) enum Step {
    Ret(Val),
    Throw(Val),
    Await(Val, Cont),
}

/// what an async function does once what it awaits settles
pub(crate) type Cont = Box<dyn FnOnce(&World, Settled) -> Step>;

/// `await v`, then k
pub(crate) fn wait(v: Val, k: impl FnOnce(&World, Settled) -> Step + 'static) -> Step {
    Step::Await(v, Box::new(k))
}

/// `await v`, then k with its value; a rejection is thrown on
pub(crate) fn wait_ok(v: Val, k: impl FnOnce(&World, Val) -> Step + 'static) -> Step {
    wait(v, move |w, r| match r {
        Ok(v) => k(w, v),
        Err(e) => Step::Throw(e),
    })
}

/// An async function: its body runs at once, to its first await, and
/// its promise settles with what it returns or throws.
pub(crate) fn spawn(w: &World, body: impl FnOnce(&World) -> Step) -> Promise {
    let p = Promise::new();
    let s = body(w);
    drive(w, p.clone(), s);
    p
}

fn drive(w: &World, p: Promise, s: Step) {
    match s {
        Step::Ret(v) => resolve(w, &p, Ok(v)),
        Step::Throw(e) => resolve(w, &p, Err(e)),
        Step::Await(v, k) => {
            let src = Promise::of(v);
            then(
                w,
                &src,
                Box::new(move |w, r| {
                    let s = k(w, r);
                    drive(w, p, s);
                    Ok(Val::Undef)
                }),
            );
        }
    }
}
