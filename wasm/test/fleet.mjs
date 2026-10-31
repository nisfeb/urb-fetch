// Engines on a loop in this process, for the tests: fake galaxies whose
// datagrams go to each other, galaxy g at 127.0.0.1:31337+g, with no
// network. Each test makes its own loop.

import { readFileSync } from 'node:fs';
import { fakeKeyfile, open } from '../mesa.mjs';

export const wasm = readFileSync(new URL('../build/mesa.wasm', import.meta.url));

export function fleet() {
  const ends = new Map(), engines = [];
  const loop = (g, sent) => ({
    start(ship, onPacket) { ends.set(g, onPacket); },
    send(lane, bytes) {
      sent?.push([lane.name ? `g${lane.galaxy}` : `${lane.ip}:${lane.port}`, Buffer.from(bytes).toString('hex')]);
      const f = ends.get(lane.name ? lane.galaxy : lane.port - 31337);
      if (f) setTimeout(() => f(0x7f000001, 31337 + g, bytes));
    },
    close() { ends.delete(g); },
  });
  return {
    // a fake galaxy on the loop; opts go to open(), and sent, if given,
    // collects every datagram it sends as [lane, hex]
    async engine(ship, g, { sent, ...opts } = {}) {
      const keyfile = await fakeKeyfile(wasm, ship);
      const urb = await open({
        wasm,
        transport: loop(g, sent),
        store: { load: () => ({ keyfile, rows: null, fresh: true }), saveKey() {}, commit() {} },
        log: () => {},
        fake: true,
        ...opts,
      });
      engines.push(urb);
      return urb;
    },
    close: () => Promise.all(engines.map(u => u.close())),
  };
}

// a fixed clock and a counting entropy source, recording each draw's size
export function fixed(start = 1_761_600_000_000) {
  let t = start, k = 0;
  const draws = [];
  return {
    draws,
    now: () => t,
    tick: ms => { t += ms; },
    entropy: n => {
      draws.push(n);
      return Uint8Array.from({ length: n }, () => (k = (k * 131 + 17) & 0xff));
    },
  };
}

// An engine whose hooks the test can call as libmesa would, with nouns it
// builds: what a hostile ship's message is once libmesa has cued it.
export async function victim() {
  const f = fleet(), clock = fixed(), logs = [];
  const real = WebAssembly.instantiate;
  let got;
  WebAssembly.instantiate = async (w, imports) => {
    const r = await real(w, imports);
    got = { host: imports.host, X: (r.instance ?? r).exports };
    return r;
  };
  let urb;
  try {
    urb = await f.engine('~zod', 0, { now: clock.now, entropy: clock.entropy, log: e => logs.push(e) });
  } finally {
    WebAssembly.instantiate = real;
  }
  const { host, X } = got;
  const nonce = [...fixed().entropy(4)].map(b => b.toString(16).padStart(2, '0')).join('');
  const atom = v => {
    const b = typeof v === 'string' ? new TextEncoder().encode(v)
      : Uint8Array.from((function* () { for (let x = BigInt(v); x > 0n; x >>= 8n) yield Number(x & 0xffn); })());
    const p = X.alloc(b.length || 1);
    new Uint8Array(X.memory.buffer).set(b, p);
    try { return X.atom(p, b.length); } finally { X.free(p); }
  };
  // a noun of a JS value: an atom, a string, an array of two or more
  // (right-associated cells), or a handle given as {n}
  const build = x => {
    if (x === null) return atom(0);
    if (x?.n !== undefined) return x.n;
    if (Array.isArray(x)) return x.slice(0, -1).reduceRight((t, h) => X.cell(build(h), t), build(x.at(-1)));
    return atom(x);
  };
  const share = (x, n) => {   // a noun that is 2^n copies of x as a tree
    let h = build(x);
    for (let i = 0; i < n; i++) h = X.cell(X.ref(h), h);
    return { n: h };
  };
  const path = s => [...s.split('/').filter(Boolean), null];
  // libmesa hands a poke from [from] (~nec by default) to the engine's
  // %grubbery hook
  const deliver = (mark, value, from = 1n) => {
    const agent = new TextEncoder().encode('grubbery\0'), p = X.alloc(agent.length);
    new Uint8Array(X.memory.buffer).set(agent, p);
    const m = atom(mark), n = build(value);
    try {
      return host.on_poke(from, 0n, 0n, p, m, n);
    } finally {
      X.free(p);
      X.unref(m);
      X.unref(n);
    }
  };
  const trapped = () => logs.filter(e => e instanceof WebAssembly.RuntimeError);
  return { f, urb, nonce, deliver, build, share, path, trapped, logs, X };
}

// p, or an error if it takes over 5 seconds (a peek gives up at 30)
export const late = p => {
  let t;
  const hung = new Promise((_, no) => { t = setTimeout(() => no(new Error('hung')), 5000); });
  return Promise.race([p, hung]).finally(() => clearTimeout(t));
};
