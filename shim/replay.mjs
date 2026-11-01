// Replays every trace through the wasm build (docs/rust-core.md, M6):
// each step's input into the core in shim/build/mesa.wasm, in node's V8,
// with the shim's own WASI imports, and its outputs compared with the
// trace's, byte for byte, as core/urb/tests/replay.rs compares them on
// the native and WASI builds. The trace's timer and untimer outputs are
// the JS's own scheduling: after each step the core's timers must be the
// ones the JS had live, by kind and due. Each step must draw exactly the
// entropy the trace says it drew.
//
//   node shim/replay.mjs [FILE.jsonl ...]

import { readdirSync, readFileSync } from 'node:fs';
import { fromLines } from '../wasm/trace.mjs';
import { instantiate } from './mesa.mjs';

const dir = new URL('../traces/', import.meta.url).pathname;
const files = process.argv.slice(2).length ? process.argv.slice(2)
  : readdirSync(dir).filter(f => f.endsWith('.jsonl')).sort((a, b) => parseInt(a) - parseInt(b)).map(f => dir + f);
const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
const module = await WebAssembly.compile(wasm);

// what the wire gives beside a trace's encoding (urb-wasm's wire.rs)
function strip(o) {
  if ('result' in o && 'err' in o) {
    const { kind, ...rest } = o;
    return rest;
  }
  if ('inbox' in o) return { inbox: o.inbox };
  return o;
}

async function replay(t) {
  let left = [], drawn = 0, wrong = null;
  const { X, text } = await instantiate(module, {
    entropy(n) {
      const h = left.shift();
      if (h === undefined || h.length !== 2 * n) {
        wrong ??= `drew ${n} bytes, the trace has ${h === undefined ? 'no more' : h.length / 2}`;
        return new Uint8Array(n);
      }
      drawn++;
      return Uint8Array.from(Buffer.from(h, 'hex'));
    },
  });
  const h = X.urb_core_new();
  const live = new Map();
  const closes = new Set();
  try {
    for (const [k, step] of t.steps.entries()) {
      const i = step.in;
      if (i.k === 'call' && i.op === 'close') closes.add(k);
      if (i.k === 'tick') live.delete(i.timer);
      left = [...i.eny];
      drawn = 0;
      const input = i.k === 'call' ? { ...i, id: k } : i;
      let r;
      try {
        r = JSON.parse(text('urb_core_step', [h, i.now], JSON.stringify(input)));
      } catch (e) {
        return { ok: false, at: k, why: `the step failed: ${e.message}` };
      }
      for (const o of step.out) {
        if ('timer' in o) live.set(o.timer, [o.kind, o.due]);
        else if ('untimer' in o) live.delete(o.untimer);
        else if ('result' in o && closes.has(o.result)) live.clear();
      }
      const key = ([kind, due]) => `${kind}\u0000${String(due).padStart(20, '0')}`;
      const js = [...live.values()].map(key).sort(), ours = r.timers.map(key).sort();
      if (JSON.stringify(js) !== JSON.stringify(ours)) {
        return { ok: false, at: k, why: `timers: the JS has ${JSON.stringify([...live.values()])}, the core ${JSON.stringify(r.timers)}` };
      }
      if (wrong) return { ok: false, at: k, why: wrong };
      if (left.length) return { ok: false, at: k, why: `${left.length} draws not made` };
      const want = step.out.filter(o => !('timer' in o) && !('untimer' in o));
      const got = r.out.map(strip);
      if (JSON.stringify(want) !== JSON.stringify(got)) {
        const n = Math.max(want.length, got.length);
        let at = 0;
        while (at < n && JSON.stringify(want[at]) === JSON.stringify(got[at])) at++;
        const cut = o => (o === undefined ? 'nothing' : JSON.stringify(o).slice(0, 300));
        return { ok: false, at: k, why: `(${i.k}) output ${at}:\n    recorded ${cut(want[at])}\n    replayed ${cut(got[at])}` };
      }
    }
  } finally {
    X.urb_core_free(h);
  }
  return { ok: true };
}

let bad = 0;
for (const f of files) {
  const t = fromLines(readFileSync(f, 'utf8'));
  const name = f.split('/').pop();
  const start = Date.now();
  const r = await replay(t);
  if (r.ok) console.log(`ok    ${name}: ${t.steps.length} steps (${Date.now() - start} ms)`);
  else {
    bad++;
    console.log(`FAIL  ${name}: step ${r.at} of ${t.steps.length}: ${r.why}`);
  }
}
process.exit(bad ? 1 : 0);
