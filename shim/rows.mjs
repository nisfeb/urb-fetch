// The saved rows, across the two modules (docs/rust-core.md, M6): rows
// saved by today's mesa.wasm (wasm/build.sh, glue.c's store, driven by
// wasm/mesa.mjs) load in the Rust one (shim/build/mesa.wasm, driven by
// shim/mesa.mjs), and the other way round, and each then carries on as
// the other would have.
//
// Each session runs both engines side by side on the same virtual
// clock, the same entropy and the same calls, each over a store kept as
// the extension keeps it in IndexedDB: a commit's rows are read with
// readRows and put or deleted by key, and a load hands back every row
// with writeRows, in the keys' binary order. Nothing reaches a network:
// the transport only records. A session opens a fake moon of ~zod on its
// store, reads a page from ~zod (nobody answers, so its keens and peeks
// run out), runs 45 virtual seconds, and closes.
//
//   1. both from empty: the same datagrams, commits and results, and the
//      same rows saved
//   2. today's module on the rows the Rust one saved, and the Rust one on
//      today's: the same again
//   3. crossed once more, from what 2 saved
//
// each of 2 and 3 sending what a start from empty would not,
//
// and, off the fake network, each refuses a moon that ran from an empty
// store, with the same words.
//
//   node shim/rows.mjs

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import * as OLD from '../wasm/mesa.mjs';
import * as NEW from './mesa.mjs';

const oldWasm = readFileSync(new URL('../wasm/build/mesa.wasm', import.meta.url));
const newWasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
const hex = b => Buffer.from(b).toString('hex');
const macrotask = () => new Promise(r => setImmediate(r));
const MOON = '~doznec-doznec-dozzod-dozzod';   // a moon of ~zod's

let failed = 0;
function check(what, ok, detail = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`);
  if (!ok) {
    failed++;
    if (detail) console.log(`     ${String(detail).slice(0, 600)}`);
  }
}

// rows as IndexedDB keeps them: by key, handed back in the keys' order
function idb(saved = new Map()) {
  const rows = new Map(saved);
  return {
    rows,
    load(m) {
      const keys = [...rows.keys()].sort((a, b) => Buffer.compare(Buffer.from(a, 'hex'), Buffer.from(b, 'hex')));
      return m.writeRows(keys.map(k => ({ key: Buffer.from(k, 'hex'), val: rows.get(k) })));
    },
    commit(m, batch) {
      for (const { key, val } of m.readRows(batch)) {
        if (val) rows.set(hex(key), val);
        else rows.delete(hex(key));
      }
    },
  };
}

// one engine's session: its datagrams, commits and results
async function session(m, wasm, keyfile, store, { fake = true, fresh = false } = {}) {
  let clock = 1_761_000_000_000, n = 0, tid = 0;
  const timers = new Map();
  const got = { sends: [], commits: [], results: [], logs: [] };
  const world = {
    wasm, fake, env: {},
    now: () => clock,
    entropy: k => new Uint8Array(createHash('sha512').update(`rows ${n++}`).digest().subarray(0, k)),
    log: e => got.logs.push(String(e?.message ?? e)),
    timers: {
      setTimeout(f, ms) {
        timers.set(++tid, { f, due: clock + Math.max(0, ms ?? 0) });
        return tid;
      },
      clearTimeout: id => timers.delete(id),
    },
    fetch: () => Promise.reject(new TypeError('offline')),
    transport: {
      start() {},
      send: (lane, bytes) => got.sends.push(`${lane.name ? `g${lane.galaxy}` : `${lane.ip}:${lane.port}`} ${hex(bytes)}`),
      close() {},
    },
    store: {
      load: () => ({ keyfile, rows: store.load(m), fresh }),
      saveKey() {},
      commit: async rows => {
        got.commits.push(hex(rows));
        store.commit(m, rows);
      },
    },
  };
  // fire each timer at its due, the earliest first, until p settles or
  // the clock passes until
  async function run(p, until) {
    let done = false;
    p?.then(() => { done = true; }, () => { done = true; });
    for (;;) {
      for (let i = 0; i < 5; i++) await macrotask();
      if (done) return;
      let next = null;
      for (const [id, t] of timers) if (!next || t.due < next[1].due) next = [id, t];
      if (!next || next[1].due > until) return;
      timers.delete(next[0]);
      clock = Math.max(clock, next[1].due);
      next[1].f();
    }
  }
  let urb;
  try {
    const opening = m.open(world);
    await run(opening, clock + 60_000);
    urb = await opening;
  } catch (e) {
    got.results.push(`open: ${e.message}`);
    return got;
  }
  got.results.push(`open: ${urb.our}`);
  const reading = urb.read('urb://~zod/rows-test').then(r => `read: ${r.body}`, e => `read: ${e.message}`);
  await run(null, clock + 45_000);
  got.results.push(await Promise.race([reading, Promise.resolve('read: unsettled')]));
  const closing = urb.close();
  await run(closing, clock + 10_000);
  await closing;
  got.results.push('closed');
  return got;
}

function same(what, a, b) {
  for (const k of ['results', 'sends', 'commits']) {
    const at = a[k].findIndex((x, i) => x !== b[k][i]);
    const ok = a[k].length === b[k].length && at < 0;
    check(`${what}: the same ${k} (${a[k].length})`, ok,
      ok ? '' : `${k} ${at < 0 ? a[k].length : at}: today's ${a[k][at] ?? 'none'}\n     Rust's ${b[k][at] ?? 'none'}`);
  }
}

const sameRows = (a, b) => a.rows.size === b.rows.size && [...a.rows].every(([k, v]) => b.rows.has(k) && hex(b.rows.get(k)) === hex(v));

const keyfile = await OLD.fakeKeyfile(oldWasm, MOON);
check('both modules make the same fake keyfile', keyfile === await NEW.fakeKeyfile(newWasm, MOON));

// 1. from empty
let a = idb(), b = idb();
const first = await session(OLD, oldWasm, keyfile, a);
same('1. from empty', first, await session(NEW, newWasm, keyfile, b));
check('1. the same rows saved', sameRows(a, b) && a.rows.size > 0, `today's ${a.rows.size} rows, Rust's ${b.rows.size}`);

// 2. and 3. crossed: each module on the rows the other saved
for (const k of [2, 3]) {
  const [fromNew, fromOld] = [idb(b.rows), idb(a.rows)];
  const before = fromNew.rows.size;
  const old = await session(OLD, oldWasm, keyfile, fromNew);
  const neu = await session(NEW, newWasm, keyfile, fromOld);
  same(`${k}. today's module on the Rust rows, the Rust one on today's`, old, neu);
  check(`${k}. the same rows saved, from ${before} loaded`, sameRows(fromNew, fromOld), `today's ${fromNew.rows.size} rows, Rust's ${fromOld.rows.size}`);
  check(`${k}. the rows loaded were used: not what a start from empty sends`, old.sends.join() !== first.sends.join());
  [a, b] = [fromNew, fromOld];
}

// off the fake network, a moon that ran may not start from empty state
const refusedOld = await session(OLD, oldWasm, keyfile, idb(), { fake: false });
const refusedNew = await session(NEW, newWasm, keyfile, idb(), { fake: false });
check('both refuse a moon that ran from an empty store, word for word',
  refusedOld.results[0] === refusedNew.results[0] && /has run before/.test(refusedOld.results[0]), `${refusedOld.results[0]}\n     ${refusedNew.results[0]}`);

console.log(failed ? `\n${failed} failed` : '\nall passed');
process.exit(failed ? 1 : 0);
