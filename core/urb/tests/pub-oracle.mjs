// The JS as the oracle for meta.pub and serving (publish.rs's tests):
// reads {cases: [{state, now, ops}]} as JSON on stdin, and for each case
// opens a fake ~zod with a fixed clock and counted entropy, publishes
// from the saved state, and runs the ops, each a second after the last:
//
//   ['put', name, body, mark?], ['remove', name], ['pages']
//   ['keep', page, from], ['drop', page, from]   a %grubbery-load of the
//                              vault file of page, from ship number from
//   ['peek', folder, name, from]
//
// It writes, for each op, its result, every save it made (as
// JSON.stringify writes them), every poke the engine sent ([mark, the
// sham of its noun]) and every line logged: [[{result, saves, pokes,
// logs}]].
//
//   node core/urb/tests/pub-oracle.mjs < in.json

import { readFileSync } from 'node:fs';
import { DESK_BASE, fakeKeyfile, open } from '../../../wasm/mesa.mjs';
import { publish } from '../../../wasm/pub.mjs';

const wasm = readFileSync(new URL('../../../wasm/build/mesa.wasm', import.meta.url));
const input = JSON.parse(readFileSync(0, 'utf8'));
const keyfile = await fakeKeyfile(wasm, '~zod');
const vault = `${DESK_BASE}/pub/vault`;
const logText = e => (e instanceof Error ? `${e.constructor.name}: ${e.message}` : String(e));
const out = [];
for (const c of input.cases) {
  let n = 0, clock = c.now;
  const pokes = [], logs = [];
  // the module's hooks and exports, as fleet.mjs's victim() takes them
  const real = WebAssembly.instantiate;
  let host, X;
  WebAssembly.instantiate = async (w, imports) => {
    const r = await real(w, imports);
    host = imports.host;
    X = { ...(r.instance ?? r).exports };
    const { poke } = X;
    const text = p => {
      const m = new Uint8Array(X.memory.buffer);
      let e = p;
      while (m[e]) e++;
      return new TextDecoder().decode(m.subarray(p, e));
    };
    X.poke = (lo, hi, bone, a, m, nn) => {
      const o = X.alloc(16);
      X.sham(nn, o);
      const h = [...new Uint8Array(X.memory.buffer, o, 16)].reverse().map(b => b.toString(16).padStart(2, '0')).join('').replace(/^0+/, '');
      X.free(o);
      pokes.push([text(m), h || '0']);
      return poke(lo, hi, bone, a, m, nn);
    };
    const inst = { exports: X };
    return r.instance ? { module: r.module, instance: inst } : inst;
  };
  let urb;
  try {
    urb = await open({
      wasm, fake: true, log: e => logs.push(logText(e)),
      transport: { start() {}, send() {}, close() {} },
      store: { load: () => ({ keyfile, rows: null, fresh: true }), saveKey() {}, commit() {} },
      now: () => clock,
      entropy: k => Uint8Array.from({ length: k }, () => n++ & 0xff),
      timers: { setTimeout: () => 0, clearTimeout() {} },
    });
  } finally {
    WebAssembly.instantiate = real;
  }
  const atom = v => {
    const b = new TextEncoder().encode(v), p = X.alloc(b.length || 1);
    new Uint8Array(X.memory.buffer).set(b, p);
    try { return X.atom(p, b.length); } finally { X.free(p); }
  };
  const build = x => (x === null ? atom('') : Array.isArray(x)
    ? x.slice(0, -1).reduceRight((t, h) => X.cell(build(h), t), build(x.at(-1)))
    : typeof x === 'number' ? (x === 0 ? atom('') : (() => { throw new Error('only 0'); })()) : atom(x));
  const path = s => [...s.split('/').filter(Boolean), null];
  const deliver = (value, from) => {
    const agent = new TextEncoder().encode('grubbery\0'), p = X.alloc(agent.length);
    new Uint8Array(X.memory.buffer).set(agent, p);
    const m = atom('grubbery-load'), nn = build(value);
    try { host.on_poke(BigInt(from), 0n, 0n, p, m, nn); } finally { X.free(p); X.unref(m); X.unref(nn); }
  };
  const load = (wire, folder, name, tag, from) => deliver([[path(wire), [0, path(folder), name]], tag, null], from);
  const saves = [];
  const pub = publish(urb, { ...c.state, save: s => saves.push(JSON.stringify(s)) });
  const steps = [];
  for (const [op, ...a] of c.ops) {
    clock += 1000;
    saves.length = 0;
    pokes.length = 0;
    logs.length = 0;
    let result;
    try {
      if (op === 'keep' || op === 'drop') result = (load('/keep/page', `${vault}/${a[0]}`, 'gmi', op, a[1]), null);
      else if (op === 'peek') result = (load('/peek/1', a[0], a[1], 'peek', a[2]), null);
      else result = JSON.stringify(op === 'put' ? pub.put(...a) : op === 'remove' ? pub.remove(...a) : pub.pages());
    } catch (e) {
      result = { error: e.message };
    }
    steps.push({ result, saves: [...saves], pokes: [...pokes], logs: [...logs] });
  }
  out.push(steps);
}
// the engines' timers never fire, so nothing ends them: exit once written
process.stdout.write(JSON.stringify(out), () => process.exit(0));
