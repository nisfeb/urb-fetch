// mesa.mjs on the Rust core (docs/rust-core.md, M6): the urb crate's
// sans-IO core, built as mesa.wasm (core/urb-wasm, shim/build.sh), with
// the module interface of wasm/mesa.mjs, so a host that imported that
// imports this unchanged.
//
// The core does the protocol: libmesa's engine, the lattice reader, the
// grubbery client and server, lattice's publishing layout, furum and
// lattice forums. This file is the host. It gives the core each input
// (an app's call, a datagram, a timer's tick, a fetch's answer) and acts
// on what it gives back:
//
//   send      datagrams to the transport, after every commit before them
//             is saved: the store may answer with a promise, and a
//             datagram a commit released waits for it, as before
//   commit    the engine's rows to the store, glue.c's [klen:4][key]
//             [vlen:4][val] as before (readRows and writeRows convert)
//   saveKey   a mined comet's keyfile; open() waits for it
//   start     the transport, for our ship; open() waits for it, and fails
//             as it fails
//   fetch     the roller's key lookups (given up after 30 s) and the comet
//             stars
//   result    a call's promise, settled with the JS's value or error
//   the rest  log lines, publishing's saves, furum's and forum's notes,
//             and grubbery pokes to our inbox
//
// The core names its timers (wake, peek <wire> and the rest) and says
// when each is due; one real timer, set for the earliest, ticks each one
// due, by name, in order.
//
// open() takes what wasm/mesa.mjs's took: {wasm, transport, store, log,
// env, fake, now, entropy, timers, fetch}, wasm being this mesa.wasm.
// The engine it gives has our, read, grub, scry, nouns, now, entropy and
// close, as before. What the core does inside, the JS no longer can:
//
//   grub.peek  takes a converter by name ('cord', 'text', 'exists',
//              'files'), or one of nouns.cord, nouns.text, nouns.exists
//              and nouns.files; a converter of the host's own can't read
//              a noun inside the module, and is refused with a TypeError
//   grub.serve, grub.changed and scry.serve
//              throw: publish() (pub.mjs) serves, inside the core
//   an inbox poke
//              hands grub.onInbox's f {from, to, mark, jam}, the noun as
//              its jam, since no noun leaves the module
//   nouns      sham and mug of a value, and the converters' names

import { call as render } from './render.js'
import { unanswered, wasi } from './wasi.js'

export const DESK_BASE = '/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app';

// The caps on another ship's nouns, which the core keeps (codec.rs)
export const MAX_CELLS = 200_000;
export const MAX_DEPTH = 64;
export const TANK_DEPTH = 32;
export const MAX_NUMBER = 32;
export const MAX_SEGS = 64;
export const MAX_KNOT = 256;
export const MAX_BYTES = 64 << 20;

const LANE_GALAXY = 1, LANE_IP = 2;
const GONE = 0xffffffff;     // a deleted row's vlen
const ROLLER = 'https://roller.urbit.org/v1/azimuth';
const ROLLER_WAIT = 30_000;

const enc = new TextEncoder(), dec = new TextDecoder();

export class UrbError extends Error {}

// ------------------------------------------------------------- the pure part

// a BigInt as @uv text, as (scot %uv n) writes it
export const uv = n => render('render_uv', [BigInt(n).toString()]);

// milliseconds since the epoch as a @da
export const da = ms => BigInt(render('render_da', [ms]));

// What an urb:// url names, as lattice's +de-urb resolves it: [ship, rel,
// mount], mount 'n', 'k', 'p', 't' or 'forum' (rel [board] or [board,
// post], post a number, or a BigInt past 2^53 - 1)
export function deUrb(url) {
  if (typeof url !== 'string') throw new TypeError('url.startsWith is not a function');
  const u = render('render_de_urb', [url], m => new UrbError(m));
  if (u.post === null) return [u.ship, u.rel, u.mount];
  const id = BigInt(u.post);
  return [u.ship, [...u.rel, id <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(id) : id], u.mount];
}

// lattice-gmi's +looks-html: the first non-blank character is <
export const looksHtml = body => render('render_looks_html', [String(body)]);

export const PAGE_MARKS = new Set(['text', 'html', 'gmi', 'md', 'js', 'css', 'noun']);

// How a page published under mark renders: 'html', 'md', 'gmi' or 'text'.
// No mark is %gmi, and only %gmi is sniffed.
export const renderAs = (mark, body) =>
  render('render_render_as', [mark === undefined || mark === null ? null : String(mark), String(body)]);

// ------------------------------------------------------------- stored rows

// rows as [{key, val}], val null for a deleted row
export function readRows(buf) {
  const v = new DataView(buf.buffer, buf.byteOffset, buf.byteLength), out = [];
  for (let at = 0; at + 8 <= buf.length;) {
    const kz = v.getUint32(at, true);
    const key = buf.slice(at + 4, at + 4 + kz);
    const vz = v.getUint32(at + 4 + kz, true);
    const val = vz === GONE ? null : buf.slice(at + 8 + kz, at + 8 + kz + vz);
    out.push({ key, val });
    at += 8 + kz + (val ? vz : 0);
  }
  return out;
}

export function writeRows(rows) {
  const out = new Uint8Array(rows.reduce((n, r) => n + 8 + r.key.length + (r.val ? r.val.length : 0), 0));
  const v = new DataView(out.buffer);
  let at = 0;
  for (const { key, val } of rows) {
    v.setUint32(at, key.length, true);
    out.set(key, at + 4);
    v.setUint32(at + 4 + key.length, val ? val.length : GONE, true);
    if (val) out.set(val, at + 8 + key.length);
    at += 8 + key.length + (val ? val.length : 0);
  }
  return out;
}

// ------------------------------------------------------------- values

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, '0'));
const hex = b => {
  let s = '';
  for (let i = 0; i < b.length; i++) s += HEX[b[i]];
  return s;
};
const unhex = h => {
  const out = new Uint8Array(h.length >> 1);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(h.substr(2 * i, 2), 16);
  return out;
};

// A JS value as JSON, and back, as trace.mjs encodes them: what the
// core's calls take and give
function enc_(v) {
  if (v === undefined) return { $u: 1 };
  if (v === null || typeof v === 'boolean' || typeof v === 'string') return v;
  if (typeof v === 'number') return Number.isFinite(v) ? v : { $num: String(v) };
  if (typeof v === 'bigint') return { $big: v.toString() };
  if (v instanceof Uint8Array) return { $hex: hex(v) };
  if (v instanceof Date) return { $date: Number.isNaN(v.getTime()) ? null : v.toISOString() };
  if (Array.isArray(v)) return v.map(enc_);
  if (v instanceof Map) return { $map: [...v].map(([k, x]) => [enc_(k), enc_(x)]) };
  if (v instanceof Set) return { $set: [...v].map(enc_) };
  if (typeof v === 'function') return { $fn: 1 };
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, enc_(x)]));
}

function dec_(v) {
  if (v === null || typeof v !== 'object') return v;
  if (Array.isArray(v)) return v.map(dec_);
  if ('$u' in v) return undefined;
  if ('$num' in v) return Number(v.$num);
  if ('$big' in v) return BigInt(v.$big);
  if ('$hex' in v) return unhex(v.$hex);
  if ('$date' in v) return new Date(v.$date ?? NaN);
  if ('$map' in v) return new Map(v.$map.map(([k, x]) => [dec_(k), dec_(x)]));
  if ('$set' in v) return new Set(v.$set.map(dec_));
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, dec_(x)]));
}

const ERRORS = { Error, TypeError, RangeError, SyntaxError, UrbError };

// an error the core gives, as the JS threw it
function errorOf(e, kind) {
  const err = e.urb ? new UrbError(e.$err) : new (ERRORS[kind] ?? Error)(e.$err);
  if (e.later) err.later = true;
  return err;
}

// ------------------------------------------------------------- the module

// an error the module answered with (code 1), not one thrown out of it
const ANSWERED = Symbol('answered');

// mesa.wasm instantiated: its exports, and text in and out of them. host
// is {log, env, entropy}.
export async function instantiate(wasm, { log = console.error, env = {}, entropy = randomBytes } = {}) {
  const module = wasm instanceof WebAssembly.Module ? wasm : await WebAssembly.compile(wasm);
  const left = unanswered(module);
  if (left.length) throw new Error(`mesa.wasm asks for WASI imports this host lacks: ${left.join(', ')}`);
  let X = null;
  const inst = await WebAssembly.instantiate(module, {
    wasi_snapshot_preview1: wasi(() => X.memory, log, env),
    host: {
      entropy(p, n) {
        const b = entropy(n);
        if (!(b instanceof Uint8Array) || b.length !== n) throw new Error(`entropy gave ${b?.length} bytes for ${n}`);
        new Uint8Array(X.memory.buffer).set(b, p);
      },
    },
  });
  X = inst.exports;
  // fn(...pre, text): its answer as text; 1 is an error, thrown as
  // err(message) and tagged as the module's answer. Anything else thrown
  // is thrown out of the module (a trap, proc_exit, a host hook's throw,
  // a stack overflow), and the module may be left half way.
  const text = (fn, pre, t, err = m => new Error(m)) => {
    const b = enc.encode(t);
    const p = X.urb_alloc(b.length);
    new Uint8Array(X.memory.buffer).set(b, p);
    const code = X[fn](...pre, p, b.length);
    const out = dec.decode(new Uint8Array(X.memory.buffer, X.urb_out_ptr(), X.urb_out_len()));
    if (code !== 0) {
      const e = err(out);
      e[ANSWERED] = true;
      throw e;
    }
    return out;
  };
  const answer = () => dec.decode(new Uint8Array(X.memory.buffer, X.urb_out_ptr(), X.urb_out_len()));
  return { X, text, answer };
}

// ------------------------------------------------------------- the relay

// Datagrams over a WebSocket to `urb-fetch -relay`, which sends and
// receives them on a UDP port it keeps for this ship. The first message
// names the ship; then each frame out is [kind:1][galaxy:1][ip:4][port:2]
// [datagram] (kind 1: to that galaxy, which the relay looks up), and each
// frame in is [ip:4][port:2][datagram], big-endian. While the socket is
// down datagrams are dropped, and the engine resends them.
export function relayTransport(url, log = console.error) {
  let ws = null, closed = false, ship, onPacket;
  const connect = () => new Promise((resolve, reject) => {
    const s = new WebSocket(url);
    let opened = false;
    s.binaryType = 'arraybuffer';
    s.onopen = () => {
      opened = true;
      ws = s;
      s.send(ship);
      resolve();
    };
    s.onmessage = e => {
      if (typeof e.data === 'string' || e.data.byteLength < 6) return;
      const v = new DataView(e.data);
      onPacket(v.getUint32(0), v.getUint16(4), new Uint8Array(e.data, 6));
    };
    s.onerror = () => {};
    s.onclose = () => {
      if (ws === s) ws = null;
      if (!opened) reject(new Error(`can't reach the relay at ${url}`));
      if (!closed) setTimeout(() => connect().catch(() => {}), 2000);
    };
  });
  return {
    start(s, f) {
      ship = s;
      onPacket = f;
      return connect().catch(e => {
        closed = true;
        throw e;
      });
    },
    send(lane, bytes) {
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      const f = new Uint8Array(8 + bytes.length);
      const v = new DataView(f.buffer);
      if (lane.name) {
        v.setUint8(0, LANE_GALAXY);
        v.setUint8(1, lane.galaxy);
      } else {
        v.setUint8(0, LANE_IP);
        v.setUint32(2, lane.ip);
        v.setUint16(6, lane.port);
      }
      f.set(bytes, 8);
      ws.send(f);
    },
    close() {
      closed = true;
      ws?.close();
    },
  };
}

// ------------------------------------------------------------- the engine

const randomBytes = n => crypto.getRandomValues(new Uint8Array(n));
const globalTimers = { setTimeout: (f, ms) => globalThis.setTimeout(f, ms), clearTimeout: t => globalThis.clearTimeout(t) };
const globalFetch = (...a) => globalThis.fetch(...a);

// the engine's calls, for pub.mjs, furum.mjs and forum.mjs: an engine
// keeps them under this key ({...urb} keeps it too)
export const CORE = Symbol('urb core');

// the converters grub.peek takes, by name
const CONV = Symbol('converter');
const conv = name => Object.assign(() => {
  throw new TypeError(`nouns.${name} reads a noun inside mesa.wasm only: hand it to grub.peek`);
}, { [CONV]: name });
const CONVS = { cord: conv('cord'), text: conv('text'), exists: conv('exists'), files: conv('files') };

export async function open({ wasm, transport, store, log = console.error, env = {}, fake = false,
                             now = Date.now, entropy = randomBytes,
                             timers = globalTimers, fetch = globalFetch }) {
  const { setTimeout, clearTimeout } = timers;
  const { X, text, answer } = await instantiate(wasm, { log, env, entropy });
  const h = X.urb_core_new();

  let dead = null;        // why the module stopped, once it has
  let ids = 0;
  const calls = new Map();   // id -> {resolve, reject} or {sync}
  const on = {};             // an event's or a note's listener, by name
  let inbox = null;          // grub.onInbox's f

  // ----------------------------------------------------------- order

  // What waits on the store and the transport: a commit's datagrams wait
  // for it to be saved, as do the transport's close and everything after
  // a start, until each settles. With a store that answers at once,
  // nothing waits.
  // Nothing runs after a failed saveKey or start: open() then throws
  // with nothing more started or saved, as before.
  let tail = null;
  function after(f, failed) {
    const run = () => {
      if (keyFailed || startFailed) return;
      try {
        return f();
      } catch (e) {
        failed(e);
      }
    };
    if (!tail) {
      const r = run();
      if (!r || typeof r.then !== 'function') return;
      const p = Promise.resolve(r).catch(failed).then(() => { if (tail === p) tail = null; });
      tail = p;
      return;
    }
    const p = tail.then(run).catch(failed).then(() => { if (tail === p) tail = null; });
    tail = p;
  }
  const settled = async () => {
    while (tail) await tail;
  };

  // ----------------------------------------------------------- stepping

  let live = [];             // the core's timers: [kind, due], in the order set

  // Anything thrown out of the module but its own answer stops it: a
  // step that didn't finish has the core taken out, and the module may be
  // half way. Every waiting call is rejected, later ones are refused, and
  // the timer stops.
  function die(e) {
    if (!dead) {
      dead = new Error(`mesa.wasm stopped: ${e?.message ?? e}`);
      for (const [, c] of calls) c.reject?.(dead);
      calls.clear();
      if (timer !== null) clearTimeout(timer);
      timer = null;
      timerDue = null;
      live = [];
    }
    return dead;
  }
  // text() on the module, its throws (but its answers) stopping it
  function guarded(fn, pre, t, err) {
    if (dead) throw dead;
    try {
      return text(fn, pre, t, err);
    } catch (e) {
      throw e?.[ANSWERED] ? e : die(e);
    }
  }

  function step(input) {
    const r = JSON.parse(guarded('urb_core_step', [h, now()], JSON.stringify(input)));
    live = r.timers;
    for (const o of r.out) act(o);
    if (!firing) arm();
  }

  // one real timer, for the earliest the core has set
  let timer = null, timerDue = null, firing = false;
  function arm() {
    const due = live.length ? Math.min(...live.map(t => t[1])) : null;
    if (due === timerDue) return;
    if (timer !== null) clearTimeout(timer);
    timer = null;
    timerDue = due;
    if (due !== null) timer = setTimeout(fire, Math.max(0, due - now()), 'core');
  }
  function fire() {
    timer = null;
    timerDue = null;
    firing = true;
    try {
      for (;;) {
        // the earliest due by now; of two due at once, the one set first
        const t = now();
        let next = null;
        for (const x of live) if (x[1] <= t && (!next || x[1] < next[1])) next = x;
        if (!next || dead) break;
        step({ k: 'tick', kind: next[0], due: next[1] });
      }
    } catch (e) {
      log(e);
    } finally {
      firing = false;
    }
    if (!dead) arm();
  }

  // galaxy number -> name, for the transport to look up
  const galaxyNames = new Map();
  const laneOf = l => {
    if (l[0] === 'g') {
      const gal = Number(l.slice(1));
      if (!galaxyNames.has(gal)) {
        X.urb_ship_name(BigInt(gal), 0n);
        galaxyNames.set(gal, answer().slice(1));
      }
      return { galaxy: gal, name: galaxyNames.get(gal) };
    }
    const [ip, port] = l.split(':').map(Number);
    return { ip, port };
  };

  let startFailed = null, keyFailed = null;
  let started = false, stopped = false;   // the transport's start and close run
  const onPacket = (ip, port, bytes) => {
    if (dead) return;
    try {
      step({ k: 'hear', ip, port, bytes: hex(bytes) });
    } catch (e) {
      log(e);
    }
  };

  function act(o) {
    if ('send' in o) {
      const lane = laneOf(o.send), bytes = unhex(o.bytes);
      after(() => transport.send(lane, bytes), log);
    } else if ('commit' in o) {
      const rows = unhex(o.commit);
      after(() => store.commit(rows), e => log(`saving the engine's state failed: ${e.message}`));
    } else if ('saveKey' in o) {
      after(() => store.saveKey(o.saveKey), e => { keyFailed = e; });
    } else if ('start' in o) {
      after(() => {
        started = true;
        return transport.start(o.start, onPacket);
      }, e => { startFailed = e; });
    } else if ('tclose' in o) {
      after(() => {
        stopped = true;
        return transport.close();
      }, log);
    } else if ('log' in o) {
      log(o.log);
    } else if ('fetch' in o) {
      fetchFor(o);
    } else if ('result' in o) {
      const c = calls.get(o.result);
      if (!c) return;
      calls.delete(o.result);
      if ('ok' in o) c.sync ? Object.assign(c, { done: true, value: dec_(o.ok) }) : c.resolve(dec_(o.ok));
      else {
        const e = errorOf(o.err, o.kind);
        c.sync ? Object.assign(c, { done: true, error: e }) : c.reject(e);
      }
    } else if ('inbox' in o) {
      try {
        inbox?.({ from: o.inbox, to: o.to, mark: o.mark, jam: unhex(o.jam) });
      } catch (e) {
        log(e);
      }
    } else if ('event' in o) {
      try {
        on[o.event]?.(dec_(o.value));
      } catch (e) {
        log(e);
      }
    }
  }

  // a fetch the core asked for; its answer is an input of its own. A key
  // lookup is given up on after 30 s, as before.
  function fetchFor(o) {
    const opts = { method: o.method, headers: o.headers, ...(o.body === null ? {} : { body: o.body }) };
    let t = null;
    if (o.url === ROLLER) {
      const ac = new AbortController();
      opts.signal = ac.signal;
      t = globalThis.setTimeout(() => ac.abort(new DOMException('The operation was aborted due to timeout', 'TimeoutError')), ROLLER_WAIT);
    }
    Promise.resolve()
      .then(() => fetch(o.url, opts))
      .then(async r => ({ status: r.status, body: hex(new Uint8Array(await r.arrayBuffer())) }))
      .catch(e => ({ error: e.message }))
      .then(got => {
        if (t !== null) globalThis.clearTimeout(t);
        if (!dead) step({ k: 'fetch', id: o.fetch, ...got });
      })
      .catch(log);
  }

  // ----------------------------------------------------------- calls

  // a call, by trace.mjs's name: a promise of its result
  function call(op, ...args) {
    return new Promise((resolve, reject) => {
      const id = ++ids;
      calls.set(id, { resolve, reject });
      try {
        step({ k: 'call', id, op, args: args.map(enc_) });
      } catch (e) {
        calls.delete(id);
        reject(e);
      }
    });
  }

  // a call the JS answered at once: its result, or its error thrown
  function callSync(op, ...args) {
    const id = ++ids, c = { sync: true };
    calls.set(id, c);
    try {
      step({ k: 'call', id, op, args: args.map(enc_) });
    } finally {
      calls.delete(id);
    }
    if (!c.done) throw new Error(`${op} did not answer at once`);
    if (c.error) throw c.error;
    return c.value;
  }

  // ----------------------------------------------------------- start

  // A failed open leaves nothing running: no engine, no timer, and no
  // transport (closed if it started and the core didn't close it).
  let our, failed = null;
  try {
    const { keyfile, rows, fresh } = await store.load();
    ({ our } = await call('open', { keyfile: keyfile ?? null, rows: rows?.length ? hex(rows) : null, fresh: Boolean(fresh) }, { fake }));
  } catch (e) {
    failed = e;
  }
  await settled();
  failed = keyFailed ?? startFailed ?? failed;
  if (failed) {
    if (started && !startFailed && !stopped) {
      try {
        transport.close();
      } catch (e) {
        log(e);
      }
    }
    const wasDead = dead;
    die(failed);
    if (!wasDead) {
      try {
        X.urb_core_free(h);
      } catch {}
    }
    throw failed;
  }

  const hash = (fn, x) => {
    try {
      return guarded(fn, [], JSON.stringify(enc_(x)));
    } catch (e) {
      if (e === dead) throw e;
      const m = /^(\w+): (.*)$/s.exec(e.message);
      throw m ? new (ERRORS[m[1]] ?? Error)(m[2]) : e;
    }
  };
  const convName = c => {
    const n = typeof c === 'string' ? c : c?.[CONV];
    if (!(n in CONVS)) throw new TypeError("grub.peek reads with 'cord', 'text', 'exists' or 'files' (or nouns.cord and the rest)");
    return n;
  };
  const serves = what => () => {
    throw new Error(`${what}: the Rust core serves through publish() (pub.mjs)`);
  };

  return {
    our,
    read: url => call('read', url),
    nouns: {
      ...CONVS,
      sham: x => BigInt(hash('urb_sham', x)),
      mug: x => Number(hash('urb_mug', x)),
    },
    now,
    entropy,
    grub: {
      peek: (ship, folder, name, c, wait) => {
        try {
          return call('peek', ship, folder, name, convName(c), wait);
        } catch (e) {
          return Promise.reject(e);
        }
      },
      poke: (ship, folder, name, mark, value) => call('poke', ship, folder, name, mark, value),
      onInbox: f => {
        inbox = f;
        callSync('inbox', Boolean(f));
      },
      serve: serves('grub.serve'),
      changed: serves('grub.changed'),
    },
    scry: { serve: serves('scry.serve') },
    // resolves once everything committed is saved and sent, and the
    // transport closed
    async close() {
      await call('close');
      await settled();
      clearTimeout(timer);
    },
    [CORE]: { call, callSync, on },
  };
}

// What a keyfile is, without opening an engine: {ship, rank, rift, life}
// (rank 0 galaxy, 1 star, 2 planet, 3 moon, 4 comet; rift null when the
// keyfile doesn't say), or null if it isn't one.
export async function keyfileInfo(wasm, keyfile) {
  const { text } = await instantiate(wasm);
  const i = JSON.parse(text('urb_keyfile_info', [], String(keyfile)));
  return i && { ship: i.ship, rank: i.rank, rift: i.rift === null ? null : BigInt(i.rift), life: BigInt(i.life) };
}

// A fake ship's keyfile (urbit -F keys, from its name), for tests that
// must not spend a real identity.
export async function fakeKeyfile(wasm, ship) {
  const { text } = await instantiate(wasm);
  return text('urb_fake_keyfile', [], String(ship), m => new UrbError(m));
}

// The @p of a ship number, as [lo, hi] BigInt halves: a moon of parent p
// is (k << 32) | p.
export async function shipName(wasm, lo, hi = 0n) {
  const { X, answer } = await instantiate(wasm);
  X.urb_ship_name(BigInt(lo), BigInt(hi));
  return answer();
}
