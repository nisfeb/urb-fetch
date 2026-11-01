//! libmesa's engine, as glue.c gives it to mesa.mjs: identities and
//! ships, the row-journaling store, and the hooks, each a trampoline
//! from C into [`World`] that catches any panic and answers as the
//! hook's default would (mesa.mjs's guarded()): a panic unwinding into
//! libmesa's C frames is undefined behaviour.

use crate::core::{Failure, World};
use mesa_sys as sys;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// A ship as glue.c hands it to JS: two 64-bit halves
pub type Ship = (u64, u64);

pub(crate) fn mship(s: Ship) -> sys::mesa_ship {
    sys::mesa_ship { lo: s.0, hi: s.1 }
}

/// a string libmesa malloc'd, taken and freed; None for NULL
unsafe fn take(p: *mut c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { sys::mesa_free(p.cast()) };
    Some(s)
}

/// text as a C string; text with a NUL in it is cut there, as
/// mesa.mjs's withStr hands it to C
pub(crate) fn cstr(s: &str) -> CString {
    let cut = s.split('\0').next().unwrap_or("");
    CString::new(cut).expect("no NUL is left")
}

/// a ship's @p
pub fn ship_text(s: Ship) -> String {
    unsafe { take(sys::mesa_ship_render(mship(s))) }.unwrap_or_default()
}

/// a ship from its @p, or None
pub fn ship_parse(text: &str) -> Option<Ship> {
    let c = cstr(text);
    let mut who = sys::mesa_ship { lo: 0, hi: 0 };
    unsafe { sys::mesa_ship_parse(c.as_ptr(), &mut who) }.then_some((who.lo, who.hi))
}

/// what a keyfile says: [ship, rift (None if it doesn't say), life]
pub fn keyfile_ship(keyfile: &str) -> Option<(Ship, Option<u64>, u64)> {
    let c = cstr(keyfile);
    let mut ide: sys::mesa_ident = unsafe { std::mem::zeroed() };
    if unsafe { sys::mesa_keyfile_parse(c.as_ptr(), &mut ide, 1) } < 1 {
        return None;
    }
    let rift = (ide.rift != sys::MESA_RIFT_UNKNOWN).then_some(ide.rift);
    Some(((ide.who.lo, ide.who.hi), rift, ide.life))
}

/// a fake ship's keyfile (urbit -F keys, from its name)
pub fn fake_keyfile(s: Ship) -> String {
    let mut ide: sys::mesa_ident = unsafe { std::mem::zeroed() };
    unsafe {
        sys::mesa_ident_fake(mship(s), &mut ide);
        take(sys::mesa_keyfile_render(&ide)).unwrap_or_default()
    }
}

/// glue.c's mine_comet: a comet mined under one of the stars in [jam]
/// (comet-stars.jam), from 64 random bytes; its keyfile, or None
pub fn mine_comet(jam: &[u8], eny: &[u8; 64]) -> Option<String> {
    mine_comet_under(&comet_stars(jam)?, eny)
}

/// the stars comet-stars.jam lists, read as the Go binding's
/// cometStars reads them, or None if it doesn't cue
pub fn comet_stars(jam: &[u8]) -> Option<Vec<u64>> {
    let one = [0u8; 1];
    let p = if jam.is_empty() {
        one.as_ptr()
    } else {
        jam.as_ptr()
    };
    let lis = unsafe { sys::mesa_cue_bytes(jam.len(), p) };
    if lis == sys::MESA_NONE {
        return None;
    }
    let mut stars = Vec::new();
    let mut t = lis;
    unsafe {
        while sys::mesa_is_cell(t) {
            let mut s = 0u64;
            sys::mesa_atom_get_u64(sys::mesa_head(t), &mut s);
            stars.push(s);
            t = sys::mesa_tail(t);
        }
        sys::mesa_unref(lis);
    }
    Some(stars)
}

/// a comet mined under one of stars (+come:dawn), from 64 random bytes:
/// its keyfile, or None if mining fails
pub fn mine_comet_under(stars: &[u64], eny: &[u8; 64]) -> Option<String> {
    let stars: Vec<sys::mesa_ship> = stars
        .iter()
        .map(|&lo| sys::mesa_ship { lo, hi: 0 })
        .collect();
    unsafe {
        let mut ide: sys::mesa_ident = std::mem::zeroed();
        if sys::mesa_comet_mine(
            stars.len(),
            stars.as_ptr(),
            eny.as_ptr(),
            u64::MAX,
            &mut ide,
        ) {
            take(sys::mesa_keyfile_render(&ide))
        } else {
            None
        }
    }
}

/// the roller's getPoint request for a ship, as JSON text
pub fn point_request(s: Ship) -> String {
    unsafe { take(sys::mesa_azimuth_point_request(mship(s))) }.unwrap_or_default()
}

// ------------------------------------------------------------ the engine

/// One open engine. Its calls are re-entrant: hooks call it again, as
/// mesa.mjs's do, so it is a raw pointer, never borrowed.
pub(crate) struct Engine {
    eng: Cell<*mut sys::mesa_engine>,
}

impl Engine {
    pub fn none() -> Engine {
        Engine {
            eng: Cell::new(std::ptr::null_mut()),
        }
    }

    pub fn is_open(&self) -> bool {
        !self.eng.get().is_null()
    }

    fn e(&self) -> Result<*mut sys::mesa_engine, Failure> {
        let e = self.eng.get();
        if e.is_null() {
            return Err(Failure::error("the engine is closed"));
        }
        Ok(e)
    }

    /// glue.c's open: as the identity in keyfile, on store; a mesa_err.
    ///
    /// # Safety
    /// `world` and `store` must outlive the engine.
    pub unsafe fn open(
        &self,
        keyfile: &str,
        now: u64,
        breached: bool,
        fake: bool,
        world: *const World,
        store: *mut sys::mesa_store,
    ) -> i32 {
        let c = cstr(keyfile);
        let mut cfg: sys::mesa_config = unsafe { std::mem::zeroed() };
        if unsafe { sys::mesa_keyfile_parse(c.as_ptr(), &mut cfg.ident, 1) } < 1 {
            return sys::mesa_err_MESA_E_ARG;
        }
        if cfg.ident.rift == sys::MESA_RIFT_UNKNOWN {
            cfg.ident.rift = 0;
        }
        cfg.store = store;
        cfg.hooks = sys::mesa_hooks {
            ctx: world as *mut c_void,
            send: Some(h_send),
            need_point: Some(h_need_point),
            plea: Some(h_plea),
            boon: Some(h_boon),
            done: Some(h_done),
            corked: Some(h_corked),
            sage: Some(h_sage),
            peek: Some(h_peek),
            breach: Some(h_breach),
            shut_key: Some(h_shut_key),
        };
        cfg.now = now;
        cfg.breached = breached;
        cfg.fake = fake;
        let mut out = std::ptr::null_mut();
        let err = unsafe { sys::mesa_engine_open(&cfg, &mut out) };
        if err == sys::mesa_err_MESA_OK {
            self.eng.set(out);
        }
        err
    }

    pub fn close(&self) {
        let e = self.eng.replace(std::ptr::null_mut());
        if !e.is_null() {
            unsafe { sys::mesa_engine_close(e) };
        }
    }

    pub fn hear(&self, ip: u32, port: u16, bytes: &[u8], now: u64) -> Result<(), Failure> {
        let lane = sys::mesa_lane {
            kind: sys::mesa_lane_kind_MESA_LANE_IP,
            galaxy: 0,
            ip,
            port,
        };
        let one = [0u8; 1];
        let p = if bytes.is_empty() {
            one.as_ptr()
        } else {
            bytes.as_ptr()
        };
        unsafe { sys::mesa_engine_hear(self.e()?, lane, p, bytes.len(), now) };
        Ok(())
    }

    pub fn wake(&self, now: u64) -> Result<(), Failure> {
        unsafe { sys::mesa_engine_wake(self.e()?, now) };
        Ok(())
    }

    /// ms, NONE for none (and for no engine)
    pub fn deadline(&self) -> u64 {
        match self.e() {
            Ok(e) => unsafe { sys::mesa_engine_deadline(e) },
            Err(_) => sys::MESA_NONE,
        }
    }

    /// glue.c's point: the roller's getPoint answer, or None for none
    pub fn point(&self, s: Ship, json: Option<&[u8]>) -> Result<bool, Failure> {
        let e = self.e()?;
        let mut pot: sys::mesa_point = unsafe { std::mem::zeroed() };
        if let Some(j) = json {
            let one = [0u8; 1];
            let p = if j.is_empty() {
                one.as_ptr()
            } else {
                j.as_ptr()
            };
            let ok =
                unsafe { sys::mesa_azimuth_point_parse(mship(s), j.len(), p.cast(), &mut pot) };
            if ok == sys::mesa_azimuth_err_MESA_AZIMUTH_OK {
                unsafe { sys::mesa_engine_point(e, &pot) };
                return Ok(true);
            }
        }
        unsafe { sys::mesa_engine_point_missing(e, mship(s)) };
        Ok(false)
    }

    /// glue.c's point_fake: a fake ship's keys, from its name
    pub fn point_fake(&self, s: Ship) -> Result<(), Failure> {
        let e = self.e()?;
        unsafe {
            let mut ide: sys::mesa_ident = std::mem::zeroed();
            let mut pot: sys::mesa_point = std::mem::zeroed();
            pot.who = mship(s);
            pot.rift = 0;
            pot.life = 1;
            pot.has_sponsor = false;
            sys::mesa_ident_fake(mship(s), &mut ide);
            sys::mesa_ident_pass(&ide, pot.pass.as_mut_ptr());
            sys::mesa_engine_point(e, &pot);
        }
        Ok(())
    }

    pub fn txn_begin(&self) -> Result<i32, Failure> {
        Ok(unsafe { sys::mesa_txn_begin(self.e()?) })
    }

    pub fn txn_commit(&self) -> Result<i32, Failure> {
        Ok(unsafe { sys::mesa_txn_commit(self.e()?) })
    }

    pub fn txn_abort(&self) {
        if let Ok(e) = self.e() {
            unsafe { sys::mesa_txn_abort(e) };
        }
    }

    /// a new forward flow: its bone, or NONE
    pub fn flow_open(&self, s: Ship) -> Result<u64, Failure> {
        let mut bone = 0u64;
        let err = unsafe { sys::mesa_flow_open(self.e()?, mship(s), &mut bone) };
        Ok(if err == sys::mesa_err_MESA_OK {
            bone
        } else {
            sys::MESA_NONE
        })
    }

    pub fn flow_cork(&self, s: Ship, bone: u64) -> Result<i32, Failure> {
        Ok(unsafe { sys::mesa_flow_cork(self.e()?, mship(s), bone) })
    }

    /// a gall poke, which consumes noun
    pub fn poke(
        &self,
        s: Ship,
        bone: u64,
        agent: &str,
        mark: &str,
        noun: crate::Noun,
    ) -> Result<i32, Failure> {
        let e = self.e()?;
        let (a, m) = (cstr(agent), cstr(mark));
        Ok(unsafe {
            sys::mesa_gall_poke(e, mship(s), bone, a.as_ptr(), m.as_ptr(), noun.into_raw())
        })
    }

    pub fn keen(&self, s: Ship, path: &str) -> Result<i32, Failure> {
        let p = cstr(path);
        Ok(unsafe { sys::mesa_keen(self.e()?, mship(s), p.as_ptr()) })
    }

    pub fn yawn(&self, s: Ship, path: &str) -> Result<i32, Failure> {
        let p = cstr(path);
        Ok(unsafe { sys::mesa_yawn(self.e()?, mship(s), p.as_ptr()) })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.close();
    }
}

// ------------------------------------------------------------ the store

/// glue.c's store: libmesa's in-memory one, noting the keys each
/// transaction writes. A commit hands the host every row those keys now
/// hold, as [klen:4][key][vlen:4][val] (little-endian, vlen UINT32_MAX
/// for a deleted row), and the host saves them before the datagrams that
/// commit released go out.
pub(crate) struct Journal {
    pub sto: sys::mesa_store,
    inn: *mut sys::mesa_store,
    dirty: RefCell<Vec<u8>>,
    txn: Cell<bool>,
    world: *const World,
}

impl Journal {
    /// a journal at a fixed address: `sto` points back at it
    pub fn new(world: *const World) -> Box<Journal> {
        let inn = unsafe { sys::mesa_store_memory() };
        let mut j = Box::new(Journal {
            sto: unsafe { std::mem::zeroed() },
            inn,
            dirty: RefCell::new(Vec::new()),
            txn: Cell::new(false),
            world,
        });
        j.sto = sys::mesa_store {
            ctx: std::ptr::null_mut(),
            begin: Some(s_begin),
            commit: Some(s_commit),
            rollback: Some(s_rollback),
            mark: Some(s_mark),
            rewind: Some(s_rewind),
            release: Some(s_release),
            get: Some(s_get),
            put: Some(s_put),
            del: Some(s_del),
            scan: Some(s_scan),
            close: Some(s_close),
        };
        // the store's ctx points back at the journal: written through the
        // pointer itself, so no later write through the Box invalidates it
        let me = &raw mut *j;
        unsafe { (*me).sto.ctx = me.cast() };
        j
    }

    pub fn store(&self) -> *mut sys::mesa_store {
        &self.sto as *const sys::mesa_store as *mut sys::mesa_store
    }

    fn inn(&self) -> &sys::mesa_store {
        unsafe { &*self.inn }
    }

    /// glue.c's store_load: put back saved rows (deleted ones skipped),
    /// unjournaled; Err if they are malformed
    pub fn load(&self, buf: &[u8]) -> Result<(), ()> {
        // every sum checked: a usize is 32 bits on wasm32, where a klen
        // near 2^32 would wrap past the bounds checks
        let end = |a: usize, b: usize| a.checked_add(b).filter(|&e| e <= buf.len()).ok_or(());
        let u32at = |at: usize| -> Result<u32, ()> {
            Ok(u32::from_le_bytes(buf[at..end(at, 4)?].try_into().unwrap()))
        };
        let mut at = 0usize;
        while at < buf.len() {
            let k = u32at(at)? as usize;
            let key = end(at, 4)?;
            let vlen = end(key, k)?;
            let v = u32at(vlen)?;
            let val = vlen + 4;
            if v == u32::MAX {
                at = val;
                continue;
            }
            let next = end(val, v as usize)?;
            let inn = self.inn();
            let put = inn.put.expect("the memory store puts");
            unsafe {
                put(
                    inn.ctx,
                    buf[key..].as_ptr(),
                    k,
                    buf[val..].as_ptr(),
                    v as usize,
                )
            };
            at = next;
        }
        Ok(())
    }

    fn dirty(&self, key: &[u8]) {
        let mut d = self.dirty.borrow_mut();
        d.extend_from_slice(&(key.len() as u32).to_le_bytes());
        d.extend_from_slice(key);
    }

    /// hand the host the rows the keys written since begin now hold
    fn flush(&self) -> i32 {
        let dir = std::mem::take(&mut *self.dirty.borrow_mut());
        let inn = self.inn();
        let get = inn.get.expect("the memory store gets");
        let mut out = Vec::new();
        let mut at = 0;
        while at < dir.len() {
            let k = u32::from_le_bytes(dir[at..at + 4].try_into().unwrap()) as usize;
            let key = &dir[at + 4..at + 4 + k];
            at += 4 + k;
            let mut val: *mut u8 = std::ptr::null_mut();
            let mut vz = 0usize;
            let got = unsafe { get(inn.ctx, key.as_ptr(), k, &mut val, &mut vz) };
            if got < 0 {
                return -1;
            }
            out.extend_from_slice(&(k as u32).to_le_bytes());
            out.extend_from_slice(key);
            if got != 0 {
                out.extend_from_slice(&u32::MAX.to_le_bytes());
            } else {
                out.extend_from_slice(&(vz as u32).to_le_bytes());
                if vz > 0 {
                    out.extend_from_slice(unsafe { std::slice::from_raw_parts(val, vz) });
                }
                unsafe { sys::mesa_free(val.cast()) };
            }
        }
        if out.is_empty() {
            return 0;
        }
        let w = unsafe { &*self.world };
        guard(w, 0, |w| w.store_commit(out))
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        unsafe { sys::mesa_store_free(self.inn) };
    }
}

unsafe fn journal<'a>(ctx: *mut c_void) -> &'a Journal {
    unsafe { &*(ctx as *const Journal) }
}

/// a store call, with a panic answered as a failure
fn stored(f: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(-1)
}

unsafe extern "C" fn s_begin(ctx: *mut c_void) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        let r = unsafe { (j.inn().begin.unwrap())(j.inn().ctx) };
        if r == 0 {
            j.txn.set(true);
            j.dirty.borrow_mut().clear();
        }
        r
    })
}

unsafe extern "C" fn s_commit(ctx: *mut c_void) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        let r = unsafe { (j.inn().commit.unwrap())(j.inn().ctx) };
        j.txn.set(false);
        if r != 0 { r } else { j.flush() }
    })
}

unsafe extern "C" fn s_rollback(ctx: *mut c_void) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        j.txn.set(false);
        j.dirty.borrow_mut().clear();
        unsafe { (j.inn().rollback.unwrap())(j.inn().ctx) }
    })
}

unsafe extern "C" fn s_mark(ctx: *mut c_void) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        unsafe { (j.inn().mark.unwrap())(j.inn().ctx) }
    })
}

unsafe extern "C" fn s_rewind(ctx: *mut c_void) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        unsafe { (j.inn().rewind.unwrap())(j.inn().ctx) }
    })
}

unsafe extern "C" fn s_release(ctx: *mut c_void) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        unsafe { (j.inn().release.unwrap())(j.inn().ctx) }
    })
}

unsafe extern "C" fn s_get(
    ctx: *mut c_void,
    key: *const u8,
    kz: usize,
    val: *mut *mut u8,
    vz: *mut usize,
) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        unsafe { (j.inn().get.unwrap())(j.inn().ctx, key, kz, val, vz) }
    })
}

/// a write outside a transaction is saved at once
unsafe extern "C" fn s_put(
    ctx: *mut c_void,
    key: *const u8,
    kz: usize,
    val: *const u8,
    vz: usize,
) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        let mut r = unsafe { (j.inn().put.unwrap())(j.inn().ctx, key, kz, val, vz) };
        if r == 0 {
            j.dirty(unsafe { std::slice::from_raw_parts(key, kz) });
            if !j.txn.get() {
                r = j.flush();
            }
        }
        r
    })
}

unsafe extern "C" fn s_del(ctx: *mut c_void, key: *const u8, kz: usize) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        let mut r = unsafe { (j.inn().del.unwrap())(j.inn().ctx, key, kz) };
        if r == 0 {
            j.dirty(unsafe { std::slice::from_raw_parts(key, kz) });
            if !j.txn.get() {
                r = j.flush();
            }
        }
        r
    })
}

unsafe extern "C" fn s_scan(
    ctx: *mut c_void,
    pre: *const u8,
    pz: usize,
    f: sys::mesa_store_scan_f,
    arg: *mut c_void,
) -> i32 {
    stored(|| {
        let j = unsafe { journal(ctx) };
        unsafe { (j.inn().scan.unwrap())(j.inn().ctx, pre, pz, f, arg) }
    })
}

/// the engine closes the store it was given: the journal frees the
/// memory store it wraps when it is dropped, so this does nothing
unsafe extern "C" fn s_close(_ctx: *mut c_void) {}

// ------------------------------------------------------------ hooks

/// A hook's body, guarded: an error is logged and a panic caught, and
/// either answers `default`, as mesa.mjs's guarded() does.
pub(crate) fn guard<T>(w: &World, default: T, f: impl FnOnce(&World) -> Result<T, Failure>) -> T {
    match catch_unwind(AssertUnwindSafe(|| f(w))) {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            w.log(e.log_text());
            default
        }
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            // the log itself may be what panicked: never unwind from here
            let _ = catch_unwind(AssertUnwindSafe(|| w.log(format!("panic: {msg}"))));
            default
        }
    }
}

/// a C string as mesa.mjs's cstr() reads it, with a TextDecoder
fn text(c: &CStr) -> String {
    crate::js::utf8(c.to_bytes())
}

unsafe fn world<'a>(ctx: *mut c_void) -> &'a World {
    unsafe { &*(ctx as *const World) }
}

unsafe extern "C" fn h_send(ctx: *mut c_void, lane: sys::mesa_lane, buf: *const u8, len: usize) {
    let w = unsafe { world(ctx) };
    let bytes = unsafe { std::slice::from_raw_parts(buf, len) }.to_vec();
    guard(w, (), |w| {
        w.hook_send(lane.kind, lane.galaxy, lane.ip, lane.port, bytes);
        Ok(())
    })
}

unsafe extern "C" fn h_need_point(ctx: *mut c_void, who: sys::mesa_ship) {
    let w = unsafe { world(ctx) };
    guard(w, (), |w| w.hook_need_point((who.lo, who.hi)))
}

/// pokes go to the host; we run no agents, so watches are flubbed
unsafe extern "C" fn h_plea(
    ctx: *mut c_void,
    her: sys::mesa_ship,
    bone: u64,
    vane: sys::mesa_noun,
    path: sys::mesa_noun,
    payload: sys::mesa_noun,
    _error: *mut sys::mesa_noun,
) -> sys::mesa_plea_result {
    let w = unsafe { world(ctx) };
    let mut req: sys::mesa_gall_request = unsafe { std::mem::zeroed() };
    match unsafe { sys::mesa_gall_request_parse(vane, path, payload, &mut req) } {
        sys::mesa_gall_request_kind_MESA_GALL_POKE => {
            let agent = text(unsafe { CStr::from_ptr(req.agent.as_ptr()) });
            guard(w, sys::mesa_plea_result_MESA_ACK, |w| unsafe {
                crate::noun::with_borrowed(req.mark, |mark| {
                    crate::noun::with_borrowed(req.noun, |noun| {
                        w.hook_poke((her.lo, her.hi), bone, &agent, mark, noun)
                    })
                })
            })
        }
        sys::mesa_gall_request_kind_MESA_GALL_WATCH
        | sys::mesa_gall_request_kind_MESA_GALL_WATCH_AS => sys::mesa_plea_result_MESA_FLUB,
        sys::mesa_gall_request_kind_MESA_GALL_LEAVE => sys::mesa_plea_result_MESA_ACK,
        _ => sys::mesa_plea_result_MESA_DROP,
    }
}

unsafe extern "C" fn h_boon(_: *mut c_void, _: sys::mesa_ship, _: u64, _: sys::mesa_noun) {}

unsafe extern "C" fn h_done(
    ctx: *mut c_void,
    her: sys::mesa_ship,
    bone: u64,
    error: sys::mesa_noun,
) {
    let w = unsafe { world(ctx) };
    guard(w, (), |w| unsafe {
        crate::noun::with_borrowed(error, |e| w.hook_done((her.lo, her.hi), bone, e))
    })
}

unsafe extern "C" fn h_corked(_: *mut c_void, _: sys::mesa_ship, _: u64) {}

unsafe extern "C" fn h_sage(
    ctx: *mut c_void,
    her: sys::mesa_ship,
    path: *const c_char,
    gage: sys::mesa_noun,
) {
    let w = unsafe { world(ctx) };
    let path = text(unsafe { CStr::from_ptr(path) });
    guard(w, (), |w| unsafe {
        crate::noun::with_borrowed(gage, |g| w.hook_sage((her.lo, her.hi), &path, g))
    })
}

/// what we publish is public: the reader doesn't change the answer
unsafe extern "C" fn h_peek(
    ctx: *mut c_void,
    _her: *const sys::mesa_ship,
    path: *const c_char,
) -> sys::mesa_noun {
    let w = unsafe { world(ctx) };
    let path = text(unsafe { CStr::from_ptr(path) });
    guard(w, sys::MESA_NONE, |w| {
        Ok(w.hook_peek(&path)?
            .map_or(sys::MESA_NONE, crate::Noun::into_raw))
    })
}

unsafe extern "C" fn h_breach(_: *mut c_void, _: sys::mesa_ship) {}

unsafe extern "C" fn h_shut_key(
    _: *mut c_void,
    _: sys::mesa_ship,
    _: *const c_char,
    _: *const c_char,
) -> u64 {
    0
}
