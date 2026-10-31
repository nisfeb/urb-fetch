// glue.c with no engine: a refused open leaves glue.c's engine NULL, and
// mesa.mjs still asks it for a deadline (arm(), after the open). Under
// wasm a NULL engine reads low memory and goes on, so with that memory
// 0xff the deadline is garbage and a wake timer is set; natively it
// would crash. Every export that takes the engine checks for none.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fakeKeyfile, open } from '../mesa.mjs';

const wasm = readFileSync(new URL('../build/mesa.wasm', import.meta.url));
const MOON = '~natdul-molwes-dozzod-dozhus';

// open() with its module's exports caught, so low memory can be filled
// before the engine opens
async function refused() {
  const real = WebAssembly.instantiate;
  let X = null;
  WebAssembly.instantiate = async (w, imports) => {
    const r = await real(w, imports);
    X = (r.instance ?? r).exports;
    return r;
  };
  const timers = [];
  try {
    const keyfile = await fakeKeyfile(wasm, MOON);
    const store = {
      load() {
        new Uint8Array(X.memory.buffer).fill(0xff, 0, 65536);
        return { keyfile, rows: null, fresh: false };
      },
      saveKey() {},
      commit() {},
    };
    const transport = { start() {}, send() {}, close() {} };
    const err = await open({
      wasm, transport, store, fake: false, log: () => {},
      timers: { setTimeout: (f, ms, what) => (timers.push(what), timers.length), clearTimeout() {} },
    }).then(() => null, e => e);
    return { err, timers, X };
  } finally {
    WebAssembly.instantiate = real;
  }
}

test('a refused open sets no timer, whatever low memory holds', async () => {
  const { err, timers } = await refused();
  assert.match(err?.message ?? '', /has run before, and its state here is empty/);
  assert.deepEqual(timers, []);
});

test('with no engine, its exports answer as for none', async () => {
  const { X } = await refused();
  const NONE = 2n ** 64n - 1n;
  assert.equal(BigInt.asUintN(64, X.deadline()), NONE);
  assert.equal(BigInt.asUintN(64, X.flow_open(0n, 0n)), NONE);
  X.wake(1n);
  X.hear(0, 0, 0, 0, 1n);
  X.point_fake(0n, 0n);
  assert.equal(X.point(0n, 0n, 0, 0), 0);
  for (const r of [X.txn_begin(), X.txn_commit(), X.flow_cork(0n, 0n, 0n), X.keen(0n, 0n, 0), X.yawn(0n, 0n, 0)]) {
    assert.equal(r, -5);   // MESA_E_STATE
  }
  X.txn_abort();
  X.close();
  assert.equal(X.our_lo(), 0n);
});

test('saved rows with a length past the buffer are refused, not wrapped', async () => {
  const { X } = await refused();
  const row = (bytes) => {
    const p = X.alloc(bytes.length);
    new Uint8Array(X.memory.buffer).set(bytes, p);
    return X.store_load(p, bytes.length);
  };
  assert.equal(row([1, 0, 0, 0, 107, 1, 0, 0, 0, 118]), 0);
  //  a klen that wraps a 32-bit size past the checks, and a vlen
  assert.equal(row([0xf8, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0]), -1);
  assert.equal(row([1, 0, 0, 0, 107, 0xfe, 0xff, 0xff, 0xff, 118]), -1);
  assert.equal(row([1, 0, 0]), -1);
});
