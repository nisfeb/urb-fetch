// trace.mjs: record a session of mesa.mjs's engine, and replay it, for
// the Rust core's parity (docs/rust-core.md, "Recorded sessions").
//
// A session is driven by its inputs alone. The recorder gives open()
// every hook it takes (the transport, the store, the log, the clock,
// the entropy, the timers and fetch) and writes a trace: one step per
// input, with the outputs that input caused.
//
//   inputs   call   an app call, {op, args}: open, read, close, a peek or
//                   poke, pub.mjs's, furum.mjs's and forum.mjs's calls
//            hear   a datagram heard, {ip, port, bytes}
//            tick   a timer firing, {timer, kind, due}: which timer, by the
//                   name mesa.mjs gives it (kind: 'wake', 'point <ship>',
//                   'roller <ship>', 'peek <wire>', 'poke <wire>', 'keen
//                   <path>', 'close') and the time it was set for. Two
//                   timers due at once fire as two ticks, in order. A
//                   tick never comes before its due: the recorder waits
//                   out a timer Node fires early. (Traces recorded before
//                   it did have early ticks, marked early: the JS finds
//                   its wake not yet due and sets it again.)
//            fetch  a fetch's answer, {id, status, body} or {id, error}:
//                   the roller's key lookups and the comet stars
//            Each step has its now (the clock, fixed for the whole step)
//            and eny, every entropy draw it made, in order.
//   outputs  send {lane, bytes}, commit {rows}, saveKey, start, tclose,
//            log, timer {id, ms, kind, due}, untimer {id}, fetch {id,
//            url, method, headers, body},
//            result {call, ok | err}, event {name, value}
//
// The step a call starts is a fresh macrotask, and so is each datagram,
// tick and fetch answer. Node runs every microtask before the next
// macrotask, and the hooks are all synchronous (the store commits at
// once, the transport is started before the engine opens it), so every
// output lands in the step whose input caused it, in record and replay
// alike.
//
// replay(trace) feeds the inputs into a fresh engine on a virtual clock
// and checks that each step's outputs equal the recorded ones, byte for
// byte (as JSON, bytes as hex). That is the recorder's own check, and
// what a Rust replay must also pass. timer and untimer are the JS's own
// scheduling: a port drives its own timers by each tick's kind and now,
// and compares the rest. replay(trace, {label: true}) gives an older
// trace's timers and ticks their kind and due, from a replay.
//
// MESA_TRACE must be unset: it makes libmesa log every packet.

import { forum } from './forum.mjs';
import { furum } from './furum.mjs';
import { UrbError, open, readRows } from './mesa.mjs';
import { publish } from './pub.mjs';

export const FORMAT = 1;

const hex = b => Buffer.from(b).toString('hex');
const unhex = h => new Uint8Array(Buffer.from(h, 'hex'));
const macrotask = () => new Promise(r => setImmediate(r));

// ------------------------------------------------------------- values

// A JS value as JSON, and back: what app calls take and give
export function enc(v) {
  if (v === undefined) return { $u: 1 };
  if (v === null || typeof v === 'boolean' || typeof v === 'string') return v;
  if (typeof v === 'number') return Number.isFinite(v) ? v : { $num: String(v) };
  if (typeof v === 'bigint') return { $big: v.toString() };
  if (v instanceof Uint8Array) return { $hex: hex(v) };
  if (v instanceof Date) return { $date: Number.isNaN(v.getTime()) ? null : v.toISOString() };
  if (v instanceof Error) return { $err: v.message, ...(v.later ? { later: true } : {}), ...(v instanceof UrbError ? { urb: true } : {}) };
  if (Array.isArray(v)) return v.map(enc);
  if (v instanceof Map) return { $map: [...v].map(([k, x]) => [enc(k), enc(x)]) };
  if (v instanceof Set) return { $set: [...v].map(enc) };
  if (typeof v === 'function') return { $fn: 1 };
  const proto = Object.getPrototypeOf(v);
  if (proto !== Object.prototype && proto !== null) return { $obj: v.constructor?.name ?? '?' };
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, enc(x)]));
}

export function dec(v) {
  if (v === null || typeof v !== 'object') return v;
  if (Array.isArray(v)) return v.map(dec);
  if ('$u' in v) return undefined;
  if ('$num' in v) return Number(v.$num);
  if ('$big' in v) return BigInt(v.$big);
  if ('$hex' in v) return unhex(v.$hex);
  if ('$date' in v) return new Date(v.$date ?? NaN);
  if ('$map' in v) return new Map(v.$map.map(([k, x]) => [dec(k), dec(x)]));
  if ('$set' in v) return new Set(v.$set.map(dec));
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, dec(x)]));
}

const laneText = l => (l.name ? `g${l.galaxy}` : `${l.ip}:${l.port}`);
const logText = e => (e instanceof Error ? `${e.constructor.name}: ${e.message}` : String(e));

// how a peek's noun comes back from a call: the converters by name
const CONVS = {
  cord: n => (n.isCell ? null : n.text()),
  // a file's text, or a know entry's body, as readPrivate reads it
  text: n => (n.isCell ? (n.head.isCell ? null : n.head.text()) : n.text()),
  exists: () => true,
  // a directory's files: [path, text or null]
  files: files => files.map(([pax, n]) => [pax, n.isCell ? null : n.text()]),
};

// ------------------------------------------------------------- a session

// The engine's world, in one of two modes. Recording, it is real: the
// transport, the clock, the entropy, timers and fetch. Replaying, it is
// the trace's.
function world(mode, { wasm, transport, onStep = () => {}, onOut = () => {} }) {
  const steps = [];
  let cur = null, clock = 0;
  let tid = 0, fid = 0;
  const live = new Map();      // timer id -> [f, real handle]
  const fetching = new Map();  // fetch id -> {resolve, reject}
  let packet = null;           // the engine's onPacket
  let replayEny = [];          // replaying: the step's draws, still to give

  const emit = o => {
    if (!cur) throw new Error('trace: an output with no step');
    cur.out.push(o);
    onOut(o);
  };
  function begin(input) {
    cur = { in: { ...input, now: mode === 'record' ? Date.now() : input.now, eny: [] }, out: [] };
    clock = cur.in.now;
    steps.push(cur);
    onStep(cur);
    return cur;
  }

  const hooks = {
    now: () => clock,
    entropy(n) {
      if (mode === 'record') {
        const b = crypto.getRandomValues(new Uint8Array(n));
        cur.in.eny.push(hex(b));
        return b;
      }
      const h = replayEny.shift();
      if (h === undefined || h.length !== 2 * n) throw new Error(`trace: drew ${n} bytes, the trace has ${h === undefined ? 'no more' : h.length / 2}`);
      cur.in.eny.push(h);
      return unhex(h);
    },
    log: e => emit({ log: logText(e) }),
    timers: {
      setTimeout(f, ms, what) {
        const id = ++tid, kind = what ?? null, due = clock + Math.max(0, ms ?? 0);
        emit({ timer: id, ms: ms ?? null, kind, due });
        const t = { f, kind, due, real: null };
        //  Node may fire a timer up to a millisecond early: wait it out
        const fire = () => {
          if (!live.has(id)) return;
          if (Date.now() < due) return void (t.real = setTimeout(fire, due - Date.now()));
          live.delete(id);
          begin({ k: 'tick', timer: id, kind, due });
          f();
        };
        if (mode === 'record') t.real = setTimeout(fire, ms);
        live.set(id, t);
        return id;
      },
      clearTimeout(id) {
        if (!live.has(id)) return;
        clearTimeout(live.get(id).real);
        live.delete(id);
        emit({ untimer: id });
      },
    },
    fetch(url, opts = {}) {
      const id = ++fid;
      emit({ fetch: id, url: String(url), method: opts.method ?? 'GET', headers: opts.headers ?? {}, body: opts.body ?? null });
      return new Promise((resolve, reject) => {
        fetching.set(id, { resolve, reject });
        if (mode !== 'record') return;
        globalThis.fetch(url, opts)
          .then(async r => ({ status: r.status, body: hex(new Uint8Array(await r.arrayBuffer())) }))
          .catch(e => ({ error: e.message }))
          .then(async got => {
            await macrotask();   // a step of its own, whatever resolved it
            begin({ k: 'fetch', id, ...got });
            answer(id, got);
          });
      });
    },
    transport: {
      start(ship, onPacket) {
        emit({ start: ship });
        packet = onPacket;
        if (mode === 'record') {
          transport.start(ship, (ip, port, bytes) => {
            begin({ k: 'hear', ip, port, bytes: hex(bytes) });
            packet(ip, port, bytes);
          });
        }
      },
      send(lane, bytes) {
        emit({ send: laneText(lane), bytes: hex(bytes) });
        if (mode === 'record') transport.send(lane, bytes);
      },
      close() {
        emit({ tclose: true });
        if (mode === 'record') transport.close();
      },
    },
  };

  // a fetch's answer as a Response would give it, read at once
  function answer(id, got) {
    const f = fetching.get(id);
    fetching.delete(id);
    if (!f) return;
    if (got.error !== undefined) return f.reject(new TypeError(got.error));
    const body = unhex(got.body);
    f.resolve({
      ok: got.status >= 200 && got.status < 300,
      status: got.status,
      arrayBuffer: async () => body.buffer.slice(body.byteOffset, body.byteOffset + body.length),
      text: async () => new TextDecoder().decode(body),
    });
  }

  // ----------------------------------------------------------- app calls

  // what the calls act on: the engine, and the apps on it
  const app = { urb: null, pub: null, furums: new Map(), forums: new Map() };
  const furumOf = host => app.furums.get(host) ?? app.furums.set(host, furum(app.urb, host)).get(host);
  const forumOf = host => app.forums.get(host) ?? app.forums.set(host, forum(app.urb, host)).get(host);
  const event = name => value => emit({ event: name, value: enc(value) });

  const OPS = {
    // load is {keyfile, rows (hex or null), fresh}; opts go to open()
    async open(load, opts = {}) {
      if (app.urb) throw new Error('trace: one engine at a time');
      const store = {
        load: () => ({ keyfile: load.keyfile ?? null, rows: load.rows ? unhex(load.rows) : null, fresh: Boolean(load.fresh) }),
        saveKey: k => emit({ saveKey: k }),
        commit: rows => emit({ commit: hex(rows) }),
      };
      app.urb = await open({ wasm, store, env: {}, ...opts, ...hooks });
      return { our: app.urb.our };
    },
    async close() {
      const urb = app.urb;
      Object.assign(app, { urb: null, pub: null, furums: new Map(), forums: new Map() });
      await urb.close();
      // the engine is gone: its timers would only touch a closed module
      for (const [id, t] of live) {
        clearTimeout(t.real);
        live.delete(id);
      }
      return true;
    },
    read: url => app.urb.read(url),
    peek: (ship, folder, name, conv, wait) => app.urb.grub.peek(ship, folder, name, CONVS[conv], wait),
    poke: (ship, folder, name, mark, value) => app.urb.grub.poke(ship, folder, name, mark, value),
    pub(state = {}) {
      app.pub = publish(app.urb, { ...state, save: event('pub.save') });
      return true;
    },
    'pub.put': (name, body, mark) => app.pub.put(name, body, mark),
    'pub.remove': name => app.pub.remove(name),
    'pub.pages': () => app.pub.pages(),
    furum: (host, method, ...args) => (method === 'onNote' ? (furumOf(host).onNote(event('furum.note')), true) : furumOf(host)[method](...args)),
    forum: (host, method, ...args) => (method === 'onNote' ? (forumOf(host).onNote(event('forum.note')), true) : forumOf(host)[method](...args)),
  };

  // an app call, as step k; its result is an output when it settles
  function call(op, args) {
    const k = steps.length - 1;
    const f = OPS[op];
    const done = v => emit({ result: k, ok: enc(v) });
    const fail = e => emit({ result: k, err: enc(e instanceof Error ? e : new Error(String(e))) });
    let p;
    try {
      if (!f) throw new Error(`trace: no call ${op}`);
      p = Promise.resolve(f(...args.map(dec)));
    } catch (e) {
      fail(e);
      return Promise.reject(e);
    }
    p.then(done, fail);
    return p;
  }

  return {
    steps, app, hooks, emit,
    // recording: an app call in its own step
    async record(op, ...args) {
      await macrotask();
      begin({ k: 'call', op, args: args.map(enc) });
      return call(op, args.map(enc));
    },
    // replaying: one recorded step's input
    apply(step) {
      begin(step.in);
      cur.in.eny = [];
      replayEny = [...step.in.eny];
      const i = step.in;
      if (i.k === 'call') return void call(i.op, i.args).catch(() => {});
      if (i.k === 'hear') {
        if (!packet) throw new Error('trace: a datagram before the transport started');
        return void packet(i.ip, i.port, unhex(i.bytes));
      }
      if (i.k === 'tick') {
        const t = live.get(i.timer);
        if (!t) throw new Error(`trace: timer ${i.timer} is not set`);
        if (i.kind !== undefined && (i.kind !== t.kind || i.due !== t.due)) {
          throw new Error(`trace: timer ${i.timer} is ${t.kind} due ${t.due}, the trace says ${i.kind} due ${i.due}`);
        }
        Object.assign(cur.in, { kind: t.kind, due: t.due }, i.now < t.due ? { early: true } : {});
        live.delete(i.timer);
        return void t.f();
      }
      if (i.k === 'fetch') return void answer(i.id, i);
      throw new Error(`trace: no input ${i.k}`);
    },
    left: () => replayEny.length,
  };
}

// ------------------------------------------------------------- record

// A recorder: rec.call(op, ...args) runs an app call as its own step and
// resolves as the call does. transport is started before the engine, so
// its start is synchronous: hear datagrams at once, before open returns.
// rec.trace(meta) is the trace. onOut hears each output as it is made.
export function recorder({ wasm, transport, onOut }) {
  if (process.env.MESA_TRACE) throw new Error('trace: unset MESA_TRACE: it changes what libmesa does');
  const w = world('record', { wasm, transport, onOut });
  return {
    call: (op, ...args) => w.record(op, ...args),
    get urb() { return w.app.urb; },
    steps: w.steps,
    // the engine's rows as saved so far, merged, for a later open
    rows() {
      const rows = new Map();
      for (const s of w.steps) {
        for (const o of s.out) {
          if (o.saveKey !== undefined) rows.clear();
          if (o.commit === undefined) continue;
          for (const r of readRows(unhex(o.commit))) {
            if (r.val) rows.set(hex(r.key), r);
            else rows.delete(hex(r.key));
          }
        }
      }
      const all = [...rows.values()], n = all.reduce((a, r) => a + 8 + r.key.length + r.val.length, 0);
      const out = new Uint8Array(n), v = new DataView(out.buffer);
      let at = 0;
      for (const { key, val } of all) {
        v.setUint32(at, key.length, true);
        out.set(key, at + 4);
        v.setUint32(at + 4 + key.length, val.length, true);
        out.set(val, at + 8 + key.length);
        at += 8 + key.length + val.length;
      }
      return n ? hex(out) : null;
    },
    trace: (meta = {}) => ({ head: { format: FORMAT, ...meta }, steps: w.steps }),
  };
}

// ------------------------------------------------------------- replay

// Replays trace into a fresh engine: each step's input, then its outputs
// compared with the recorded ones. {ok, steps, at, want, got} with at the
// first step that differs (and what it had, and what replay gave). With
// label, a trace from before timers had names may lack each timer's kind
// and due, and each fetch's headers: they are taken from the replay, and
// trace is the trace with them.
export async function replay(trace, { wasm, patience = 10_000, label = false } = {}) {
  const w = world('replay', { wasm });
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  // what of a replayed output the recorded one has, when labelling
  const LABELS = { timer: ['kind', 'due'], fetch: ['headers'] };
  const like = (o, rec) => {
    if (!label || !rec) return o;
    const k = Object.keys(LABELS).find(k => k in o && k in rec);
    if (!k) return o;
    const c = { ...o };
    for (const f of LABELS[k]) if (!(f in rec)) delete c[f];
    return c;
  };
  const check = (k, step, got) => {
    if (w.left()) got.out.push({ trace: `${w.left()} draws not made` });
    const outs = got.out.map((o, i) => like(o, step.out[i]));
    const inOf = i => (label ? { ...i, kind: undefined, due: undefined, early: undefined } : i);
    if (!same(outs, step.out) || !same(got.in.eny, step.in.eny) || !same(inOf(got.in), inOf(step.in))) {
      return { ok: false, steps: trace.steps.length, at: k, want: step, got };
    }
    return null;
  };
  for (const [k, step] of trace.steps.entries()) {
    let got;
    try {
      w.apply(step);
      got = w.steps[k];
      // the step's outputs: everything up to a macrotask after the last
      // one the trace has (opening the module takes real time)
      const until = Date.now() + patience;
      while (got.out.length < step.out.length && Date.now() < until) await macrotask();
      for (let i = 0; i < 3; i++) await macrotask();
    } catch (e) {
      return { ok: false, steps: trace.steps.length, at: k, want: step, got: { error: logText(e) } };
    }
    const bad = check(k, step, got);
    if (bad) return bad;
  }
  // nothing more after the last step
  for (let i = 0; i < 20; i++) await macrotask();
  const last = trace.steps.length - 1;
  if (last >= 0 && w.steps[last].out.length !== trace.steps[last].out.length) {
    return { ok: false, steps: trace.steps.length, at: last, want: trace.steps[last], got: w.steps[last] };
  }
  if (w.app.urb) await w.app.urb.close().catch(() => {});
  return { ok: true, steps: trace.steps.length, ...(label ? { trace: { head: trace.head, steps: w.steps } } : {}) };
}

// The first output where two steps' differ, as text
export function difference(want, got) {
  if (got.error) return `replay failed: ${got.error}`;
  const a = want.out, b = got.out;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = JSON.stringify(a[i]), y = JSON.stringify(b[i]);
    if (x !== y) return `output ${i}: recorded ${x?.slice(0, 300)}\n  replayed ${y?.slice(0, 300)}`;
  }
  return `entropy: recorded ${JSON.stringify(want.in.eny)}, replayed ${JSON.stringify(got.in.eny)}`;
}

// ------------------------------------------------------------- files

// A trace file is JSON lines: the head, then one step per line
export const toLines = t => [JSON.stringify(t.head), ...t.steps.map(s => JSON.stringify(s))].join('\n') + '\n';
export function fromLines(text) {
  const [head, ...steps] = text.split('\n').filter(Boolean).map(l => JSON.parse(l));
  if (head.format !== FORMAT) throw new Error(`trace: format ${head.format}, not ${FORMAT}`);
  return { head, steps };
}

// ------------------------------------------------------------- drift

// What a trace says at the level of its app calls, the same in every
// recording of the same session against the same fleet snapshot: each
// call with its result and the events it brought, in order. What changes
// from one recording to the next is masked: dates, @uv ids and times in
// ms (our page ids are entropy, a rotated salt is too), a mined comet's
// name, keyfiles and rows; and a run of the same call (a poll until the
// host shows something) is its last. Re-recording compares these.
export function digest(trace) {
  const mask = v => {
    if (typeof v === 'string') {
      if (/^0v[0-9a-v.]+$/.test(v)) return '<uv>';
      if (/^0w/.test(v) || /^[0-9a-f]{64,}$/.test(v)) return '<bytes>';
      return v.replace(/~[a-z]{6}(-[a-z]{6})*--[a-z]{6}(-[a-z]{6})*/g, '<comet>');
    }
    if (typeof v === 'number') return v > 1e12 ? '<ms>' : v;
    if (Array.isArray(v)) return v.map(mask);
    if (v && typeof v === 'object') {
      if ('$date' in v) return '<date>';
      if ('$hex' in v) return '<bytes>';
      return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, k === 'rows' || k === 'keyfile' ? '<bytes>' : mask(x)]));
    }
    return v;
  };
  const calls = [];
  const byStep = new Map();
  for (const [k, s] of trace.steps.entries()) {
    if (s.in.k === 'call') byStep.set(k, calls.push({ call: mask([s.in.op, ...s.in.args]), events: [] }) - 1);
    for (const o of s.out) {
      if (o.result !== undefined && byStep.has(o.result)) Object.assign(calls[byStep.get(o.result)], mask(o.ok !== undefined ? { ok: o.ok } : { err: o.err }));
      if (o.event && calls.length) calls.at(-1).events.push(mask([o.event, o.value]));
    }
  }
  const out = [];
  for (const c of calls) {
    const prev = out.at(-1);
    if (prev && JSON.stringify(prev.call) === JSON.stringify(c.call) && !prev.events.length) out.pop();
    out.push(c);
  }
  return out;
}
