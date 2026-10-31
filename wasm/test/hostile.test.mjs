// Hostile nouns from other ships, written by hand: deep nesting, huge
// declared sizes, shared structure and wrong shapes. Each must give an
// error, quickly, and never trap the module or hang.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DESK_BASE, MAX_CELLS, UrbError, decoders } from '../mesa.mjs';
import { furum } from '../furum.mjs';
import { publish } from '../pub.mjs';
import { fixed, fleet } from './fleet.mjs';

// ------------------------------------------------------------ the decoders

// nouns in plain JS, so shared structure can be as vast as we like,
// with mesa.mjs's Noun's checks
class Fake {
  constructor(h, t, v) { Object.assign(this, { h, t, v }); }
  get isCell() { return this.v === undefined; }
  get head() { if (!this.isCell) throw new UrbError('atom'); return this.h; }
  get tail() { if (!this.isCell) throw new UrbError('atom'); return this.t; }
  get size() { if (this.isCell) throw new UrbError('cell'); return this.bytes().length; }
  bytes() {
    if (this.isCell) throw new UrbError('cell');
    if (typeof this.v === 'string') return new TextEncoder().encode(this.v);
    const b = [];
    for (let x = BigInt(this.v); x > 0n; x >>= 8n) b.push(Number(x & 0xffn));
    return Uint8Array.from(b);
  }
  text() { return new TextDecoder().decode(this.bytes()); }
  int(max = 32) {
    if (this.size > max) throw new UrbError(`a number over ${max} bytes`);
    return this.bytes().reduceRight((v, x) => (v << 8n) | BigInt(x), 0n); }
}
const A = v => new Fake(undefined, undefined, v);
const C = (h, t) => new Fake(h, t);
const nil = A(0);
// a cell that is 2^n cells as a tree, in n+1 cells
const shared = n => { let x = A(1); for (let i = 0; i < n; i++) x = C(x, x); return x; };
const quick = (f, ms = 2000) => {
  const t = Date.now();
  const r = f();
  assert.ok(Date.now() - t < ms, `took ${Date.now() - t} ms`);
  return r;
};
const refused = (f, re) => quick(() => assert.throws(f, e => e instanceof UrbError && (!re || re.test(e.message))));

test('measure counts shared structure as a tree', () => {
  const fits = shared(17);   // 131,071 cells
  assert.equal(quick(() => decoders.measure(fits)), fits);
  refused(() => decoders.measure(shared(18)), /over 200000 cells/);
  refused(() => decoders.measure(shared(200)), /over 200000 cells/);
  //  deep in either direction: no recursion to overflow
  let deep = A(1), wide = nil;
  for (let i = 0; i < MAX_CELLS; i++) [deep, wide] = [C(deep, nil), C(A(1), wide)];
  quick(() => decoders.measure(deep));
  quick(() => decoders.measure(wide));
  refused(() => decoders.measure(C(deep, nil)));
});

test('json: depth, size, shared structure and wrong tags', () => {
  const arr = (...xs) => C(A('a'), xs.reduceRight((t, x) => C(x, t), nil));
  const num = n => C(A('n'), A(String(n)));
  assert.deepEqual(decoders.json(arr(num(1), C(A('s'), A('x')), C(A('b'), A(0)))), [1, 'x', true]);
  let deep = num(1);
  for (let i = 0; i < 63; i++) deep = arr(deep);
  assert.equal(decoders.json(deep).flat(Infinity)[0], 1);
  refused(() => decoders.json(arr(deep)), /nested over 64/);
  for (let i = 0; i < 100_000; i++) deep = arr(deep);
  refused(() => decoders.json(deep));
  //  an array of two of the same, 40 times: 2^40 values in 80 cells
  let wide = num(1);
  for (let i = 0; i < 40; i++) wide = arr(wide, wide);
  refused(() => decoders.json(wide), /cells/);
  refused(() => decoders.json(C(A('x'), A(1))), /no kind/);
  refused(() => decoders.json(C(A('b'), A(2n ** 512n))), /number over 32 bytes/);
  refused(() => decoders.json(C(C(A(1), A(2)), A(1))));   // a cell for a tag
  //  an object whose map is shared: 2^30 entries in 60 cells
  let map = nil;
  for (let i = 0; i < 30; i++) map = C(C(A('k'), num(1)), C(map, map));
  refused(() => decoders.json(C(A('o'), map)));
  refused(() => decoders.entries(map), /map over/);
});

test('list, paths and tangs', () => {
  //  a list can't be shared into length, but it can be long
  let long = nil;
  for (let i = 0; i <= MAX_CELLS; i++) long = C(A(97), long);
  refused(() => [...decoders.list(long)], /list over/);
  const path = segs => segs.reduceRight((t, s) => C(A(s), t), nil);
  assert.equal(decoders.pathOf(path(['a', 'b'])), '/a/b');
  refused(() => decoders.pathOf(path(Array(65).fill('a'))), /over 64 segments/);
  refused(() => decoders.pathOf(path(['x'.repeat(257)])), /over 256 bytes/);
  //  a tang: [%leaf tape], and a rose of the same rose 40 deep, which
  //  printed as a tree would be 2^32 leaves
  const tape = s => [...s].reduceRight((t, c) => C(A(c.charCodeAt(0)), t), nil);
  const leaf = C(A('leaf'), tape('hi'));
  assert.equal(decoders.tang(C(A('err'), C(leaf, nil))), 'err\n  hi');
  let rose = leaf;
  for (let i = 0; i < 40; i++) rose = C(A('rose'), C(C(tape(' '), C(nil, nil)), C(rose, C(rose, nil))));
  assert.match(quick(() => decoders.tang(C(A('err'), C(rose, nil)))), /over 200000 cells/);
  assert.equal(decoders.tang(A(5)), '');
  //  a tank of the wrong shape prints as nothing, or says why
  assert.equal(typeof decoders.tang(C(A('err'), C(C(A('rose'), A(1)), nil))), 'string');
});

test('fileOf: a silo of the wrong shape', () => {
  const snap = C(nil, C(C(A('firm'), C(nil, A(7))), nil));
  assert.equal(decoders.fileOf(snap, C(A(1), A(2)), () => 1), null);   // a silo of atoms holds nothing
  refused(() => decoders.fileOf(snap, C(nil, C(C(A(2n ** 300n), nil), C(nil, nil))), () => 1), /number/);
  refused(() => decoders.fileOf(C(nil, C(C(C(A(1), A(2)), nil), nil)), nil, () => 1));   // a cell for the pace
});

// ------------------------------------------------------------ the engine

// An engine whose hooks the test can call as libmesa would, with nouns it
// builds: what a hostile ship's message is once libmesa has cued it.
async function victim() {
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
  // libmesa hands a %grubbery-transfer from ~nec to the engine's hook
  const deliver = (mark, value) => {
    const agent = new TextEncoder().encode('grubbery\0'), p = X.alloc(agent.length);
    new Uint8Array(X.memory.buffer).set(agent, p);
    const m = atom(mark), n = build(value);
    try {
      return host.on_poke(1n, 0n, 0n, p, m, n);
    } finally {
      X.free(p);
      X.unref(m);
      X.unref(n);
    }
  };
  const trapped = () => logs.filter(e => e instanceof WebAssembly.RuntimeError);
  return { f, urb, nonce, deliver, share, path, trapped, logs };
}

// p, or an error if it takes over 5 seconds (a peek gives up at 30)
const late = p => {
  let t;
  const hung = new Promise((_, no) => { t = setTimeout(() => no(new Error('hung')), 5000); });
  return Promise.race([p, hung]).finally(() => clearTimeout(t));
};

test('hostile answers to a peek fail it, and nothing traps', { timeout: 60_000 }, async () => {
  const v = await victim();
  let k = 0;
  const peekWith = async (answer, conv = v.urb.nouns.json) => {
    const p = v.urb.grub.peek('~nec', '/x', 'y', conv);
    const wire = v.path(`/urb-fetch/${v.nonce}/${++k}`);
    v.deliver('grubbery-transfer', answer(wire));
    return late(p);
  };
  //  a snap answer: [wire %snap dest id snap=(unit) data=(unit silo)]
  const snapOf = (wire, value, { id = 5, pace = ['firm', null, 7] } = {}) =>
    [wire, 'snap', null, id, [null, [null, pace, null]],
     [null, [[[9, null, null], null, null], [[7, null, 'leaf', 9, null], null, null]]]];
  const file = (wire, value) => {
    const s = snapOf(wire);
    s[5][1][0][0] = [9, null, value];
    return s;
  };
  //  sanity: a well-formed answer reads
  assert.deepEqual(await peekWith(w => file(w, ['a', [['n', '1'], null]])), [1]);
  const refusedBy = async (answer, re, conv) => {
    const e = await peekWith(answer, conv).then(() => null, e => e);
    assert.ok(e instanceof UrbError, `not refused: ${e}`);
    if (re) assert.match(e.message, re);
  };
  await refusedBy(w => file(w, v.share(['n', '1'], 40)), /over 200000 cells/);
  await refusedBy(w => [w, 'snap', null, v.share(1, 30), null, null], /cells/);
  let deep = ['n', '1'];
  for (let i = 0; i < 200; i++) deep = ['a', [deep, null]];
  await refusedBy(w => file(w, deep), /nested over 64/);
  await refusedBy(w => file(w, ['q', 'x']), /no kind/);
  await refusedBy(w => snapOf(w, null, { id: 2n ** 400n }).map((x, i) => (i === 5 ? null : x)), /number over 32/);
  await refusedBy(w => snapOf(w, null, { pace: [['firm', 'x'], null, 7] }), /cell where an atom/);
  await refusedBy(w => [w, 'snap', 5], /atom where a cell/);
  await refusedBy(w => [w, 'snap', null, 5, [null, [null, 'firm']], [null, 1]], /atom where a cell/);
  await refusedBy(w => [w, ['snap', 1], 1], /cell where an atom/);
  //  a %veto or %miss is nothing there
  assert.equal(await peekWith(w => [w, 'veto', null]), null);
  assert.equal(await peekWith(w => [w, 'miss', null]), null);
  //  a message whose wire can't be read waits on nothing, and is dropped
  v.deliver('grubbery-transfer', v.share(1, 40));
  v.deliver('grubbery-transfer', 7);
  v.deliver('grubbery-load', v.share(1, 40));
  v.deliver('grubbery-load', [[v.path('/w'), 0, v.path('/a'), 'b'], 'poke', 1]);
  assert.deepEqual(v.trapped(), []);
  //  and the engine still works
  assert.deepEqual(await peekWith(w => file(w, ['s', 'still'])), 'still');
  await v.f.close();
});

test('a hostile nack prints, bounded', { timeout: 30_000 }, async () => {
  const v = await victim();
  const p = v.urb.grub.poke('~nec', '/x', 'y', ['/furum', 'msg'], 1);
  //  [%pack err=(unit tang)] with a tang that is vast as a tree
  v.deliver('grubbery-transfer', [v.path(`/grub-poke/${v.nonce}/1`), 'pack', null, v.share(['leaf', [104, null]], 40), null]);
  const r = await late(p);
  assert.equal(r.ok, false);
  assert.match(r.error, /over 200000 cells/);
  assert.deepEqual(v.trapped(), []);
  await v.f.close();
});

// ------------------------------------------------------------ over the wire

test('a hostile publisher and a hostile furum host, over the loop', { timeout: 60_000 }, async () => {
  const f = fleet();
  const bad = await f.engine('~zod', 0), us = await f.engine('~nec', 1);
  let deep = ['n', '1'];
  for (let i = 0; i < 100; i++) deep = ['a', [deep, null]];
  const fb = '/apps/shell.shell/desks/furum.desk/desk/data/furum.furum_app/boards/b';
  const files = {
    [`${DESK_BASE}/pub revs`]: deep,   // a revision list nested 100 deep
    [`${fb}/pub conf`]: [1, 2n ** 200n, null],   // a board claiming 2^200 posts
    [`${fb}/content/posts b0`]: [1, 'not', 'a map'],
  };
  bad.grub.serve((folder, name) => {
    const v = files[`${folder} ${name}`];
    return v === undefined ? null : { mark: ['/', 'noun'], value: v, rev: 1, at: 0 };
  });
  const e = await late(us.read('urb://~zod/page').then(() => null, e => e));
  assert.ok(e instanceof UrbError && /nested over 64/.test(e.message), String(e));
  const board = furum(us, '~zod');
  const p = await late(board.posts('b').then(() => null, e => e));
  assert.ok(p instanceof UrbError && /over 100000/.test(p.message), String(p));
  files[`${fb}/pub conf`] = [1, 3, null];
  const q = await late(board.posts('b').then(() => null, e => e));
  assert.ok(q instanceof UrbError, String(q));
  //  and once ~zod publishes properly, ~nec reads it
  publish(bad, {}).put('fine', 'ok');
  const r = await late(us.read('urb://~zod/fine'));
  assert.equal(r.body, 'ok');
  await f.close();
});
