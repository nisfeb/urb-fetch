// The shims' failure paths (docs/rust-core.md, M6's fix round), on
// shim/build/mesa.wasm and render.wasm, with nothing on a network:
//
//   - anything thrown out of the module, not only a trap (a throwing log
//     reached through fd_write, a throwing entropy), stops it: waiting
//     calls reject with `mesa.wasm stopped: ...`, later calls fail at once
//     with the same, and the timer is not set again
//   - a failed open() leaves nothing running: a failed saveKey starts no
//     transport, and a failed start, torn rows or a module that stops
//     while opening leave no timer, and no transport the core started
//   - render.wasm compiles on its first call: a failure throws there,
//     not at import
//
//   node shim/fail.mjs

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fromLines } from '../wasm/trace.mjs';
import * as M from './mesa.mjs';

const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
const MOON = '~doznec-doznec-dozzod-dozzod';
const macrotask = () => new Promise(r => setImmediate(r));
const keyfile = await M.fakeKeyfile(wasm, MOON);

let failed = 0;
function check(what, ok, detail = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`);
  if (!ok) {
    failed++;
    if (detail) console.log(`     ${String(detail).slice(0, 600)}`);
  }
}

// a world on a virtual clock that records what the engine started
function world(over = {}) {
  let n = 0, tid = 0;
  const w = {
    clock: 1_761_000_000_000, timers: new Map(), sets: 0,
    started: 0, closed: 0, sends: 0, commits: 0, logs: [],
  };
  w.opts = {
    wasm, fake: true, env: {},
    now: () => w.clock,
    entropy: k => new Uint8Array(createHash('sha512').update(`fail ${n++}`).digest().subarray(0, k)),
    log: e => w.logs.push(String(e?.message ?? e)),
    timers: {
      setTimeout(f, ms) {
        w.sets++;
        w.timers.set(++tid, { f, due: w.clock + Math.max(0, ms ?? 0) });
        return tid;
      },
      clearTimeout: id => w.timers.delete(id),
    },
    fetch: () => Promise.reject(new TypeError('offline')),
    transport: {
      start: () => { w.started++; },
      send: () => { w.sends++; },
      close: () => { w.closed++; },
    },
    store: {
      load: () => ({ keyfile, rows: null, fresh: true }),
      saveKey() {},
      commit: () => { w.commits++; },
    },
    ...over,
  };
  // fire each timer at its due, the earliest first, up to max of them
  w.run = async (max = 200) => {
    for (let i = 0; i < max; i++) {
      await macrotask();
      let next = null;
      for (const [id, t] of w.timers) if (!next || t.due < next[1].due) next = [id, t];
      if (!next) return;
      w.timers.delete(next[0]);
      w.clock = Math.max(w.clock, next[1].due);
      next[1].f();
    }
  };
  return w;
}

const settle = (p, w) => Promise.race([
  p.then(v => ({ v }), e => ({ e })),
  w.run().then(() => ({ pending: true })),
]);

// --------------------------------------------- a throw out of the module

// (an entropy that throws is below: the core draws entropy as it opens)
{
  const what = 'a log that throws, reached through fd_write';
  const w = world({ env: { MESA_TRACE: '1' } });
  let bad = false;
  const log = w.opts.log;
  w.opts.log = e => {
    if (bad) {
      bad = false;
      throw new Error('it broke');
    }
    log(e);
  };
  const urb = await M.open(w.opts);
  bad = true;
  // a read sends at once, and libmesa logs the datagram under MESA_TRACE
  const r = await settle(urb.read('urb://~zod/n/x'), w);
  check(`${what}: the waiting read rejects as the module stopped`,
    r.e && /^mesa\.wasm stopped: it broke/.test(r.e.message), JSON.stringify(r.e?.message ?? r));
  const setsThen = w.sets;
  await w.run();
  check(`${what}: no timer is left or set again`, w.timers.size === 0 && w.sets === setsThen,
    `${w.timers.size} live, ${w.sets - setsThen} set since`);
  check(`${what}: no "no core" spin`, !w.logs.some(l => /no core/.test(l)), w.logs.filter(l => /no core/.test(l)).length);
  const later = await settle(urb.read('urb://~zod/n/y'), w);
  check(`${what}: a later call fails at once, with the same words`, later.e?.message === r.e?.message, later.e?.message);
}

// --------------------------------------------- a failed open

// a failed saveKey: open throws it, and starts nothing (trace 1 mines a
// comet, saves its key and starts the transport in one step)
{
  const t = fromLines(readFileSync(new URL('../traces/1-cold-comet.jsonl', import.meta.url), 'utf8'));
  const body = Buffer.from(t.steps[1].in.body, 'hex');
  const eny = t.steps.flatMap(s => s.in.eny);
  const w = world();
  w.opts.fake = false;
  w.opts.entropy = n => {
    const h = eny.shift();
    return h && h.length === 2 * n ? Uint8Array.from(Buffer.from(h, 'hex')) : new Uint8Array(n).fill(3);
  };
  w.opts.fetch = async () => new Response(body, { status: 200 });
  w.opts.store = {
    load: () => ({ keyfile: null, rows: null, fresh: false }),
    saveKey: async () => { throw new Error('disk full'); },
    commit: async () => { w.commits++; },
  };
  let e = null;
  try {
    await M.open(w.opts);
  } catch (x) {
    e = x;
  }
  await w.run();
  check('a failed saveKey: open throws it', e?.message === 'disk full', e?.message);
  check('a failed saveKey: no transport started, nothing sent or saved after it',
    w.started === 0 && w.sends === 0 && w.commits === 0, `started ${w.started}, sends ${w.sends}, commits ${w.commits}`);
  check('a failed saveKey: no timer left', w.timers.size === 0, `${w.timers.size} live`);
}

// a failed start
{
  const w = world();
  w.opts.transport.start = async () => {
    w.started++;
    throw new Error("can't reach the relay");
  };
  let e = null;
  try {
    await M.open(w.opts);
  } catch (x) {
    e = x;
  }
  const sets = w.sets;
  await w.run();
  check('a failed start: open throws it', e?.message === "can't reach the relay", e?.message);
  check('a failed start: no timer left or set, nothing sent after it',
    w.timers.size === 0 && w.sets === sets && w.sends === 0, `${w.timers.size} live, sends ${w.sends}`);
}

// torn rows
{
  const w = world();
  const rows = M.writeRows([{ key: new Uint8Array([1, 2, 3]), val: new Uint8Array([4, 5, 6, 7]) }]);
  w.opts.store.load = () => ({ keyfile, rows: rows.subarray(0, rows.length - 1), fresh: false });
  let e = null;
  try {
    await M.open(w.opts);
  } catch (x) {
    e = x;
  }
  await w.run();
  check('torn rows: open throws', /store_load failed/.test(e?.message), e?.message);
  check('torn rows: no timer left, and no transport left started',
    w.timers.size === 0 && w.started === w.closed, `${w.timers.size} live, started ${w.started}, closed ${w.closed}`);
}

// a module that stops while opening
{
  const w = world();
  w.opts.entropy = () => { throw new Error('no entropy'); };
  let e = null;
  try {
    await M.open(w.opts);
  } catch (x) {
    e = x;
  }
  await w.run();
  check('stopped while opening: open throws as the module stopped',
    /^mesa\.wasm stopped: no entropy/.test(e?.message), e?.message);
  check('stopped while opening: no timer left, and no transport left started',
    w.timers.size === 0 && w.started === w.closed, `${w.timers.size} live, started ${w.started}, closed ${w.closed}`);
}

// --------------------------------------------- render.wasm, compiled late

{
  const Module = WebAssembly.Module;
  WebAssembly.Module = function () { throw new Error('wasm is off'); };
  let imported = null, first = null, second = null;
  try {
    imported = await import('./render.js?off');
    try { imported.call('render_looks_html', [' <p>']); } catch (e) { first = e; }
    try { imported.call('render_looks_html', [' <p>']); } catch (e) { second = e; }
  } catch (e) {
    imported = e;
  } finally {
    WebAssembly.Module = Module;
  }
  check('render.wasm: importing it compiles nothing', typeof imported?.call === 'function', imported?.message);
  check('render.wasm: a failed compile throws at the call, and at each after',
    /wasm is off/.test(first?.message) && second?.message === first?.message, `${first?.message} / ${second?.message}`);
  const { call } = await import('./render.js');
  check('render.wasm: compiled on its first call, it renders', call('render_looks_html', [' <p>']) === true);
}

console.log(failed ? `\n${failed} failed` : '\nall passed');
process.exit(failed ? 1 : 0);
