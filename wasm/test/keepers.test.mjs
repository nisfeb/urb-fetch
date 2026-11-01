// The keeps other ships' grubbery makes of our files. A keep may wait
// for a page not yet published, or one taken down, as lattice's reader
// keeps it, and a later publish reaches it. Each ship has a share of the
// slots, so one can't take them all; a repeated keep saves nothing; the
// cap holds; and one ship's drop leaves another's keep.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DESK_BASE } from '../mesa.mjs';
import { publish } from '../pub.mjs';
import { victim } from './fleet.mjs';

const vault = `${DESK_BASE}/pub/vault`;
const NEC = 1n, BUD = 2n;

// a %grubbery-load of the vault file of [page], from [from]
const load = (v, page, tag, from) =>
  v.deliver('grubbery-load', [[v.path('/keep/page'), [0, v.path(`${vault}/${page}`), 'gmi']], tag, null], from);

// a victim publishing, and every grubbery poke its engine makes, as
// [mark, ship number]
async function serving(t) {
  const pokes = [];
  const real = WebAssembly.instantiate;
  WebAssembly.instantiate = async (w, imports) => {
    const r = await real(w, imports);
    const X = { ...(r.instance ?? r).exports };
    const { poke } = X;
    X.poke = (lo, hi, bone, a, m, n) => {
      const b = new Uint8Array(X.memory.buffer);
      let e = m;
      while (b[e]) e++;
      pokes.push([new TextDecoder().decode(b.subarray(m, e)), lo]);
      return poke(lo, hi, bone, a, m, n);
    };
    const inst = { exports: X };
    return r.instance ? { module: r.module, instance: inst } : inst;
  };
  let v;
  try {
    v = await victim();
  } finally {
    WebAssembly.instantiate = real;
  }
  t.after(() => v.f.close());
  const saves = [];
  const pub = publish(v.urb, { save: s => saves.push(JSON.parse(JSON.stringify(s))) });
  // the waves sent to [ship] by f
  const waves = (ship, f) => {
    const n = pokes.length;
    f();
    return pokes.slice(n).filter(([m, lo]) => m === 'grubbery-intake' && lo === ship).length;
  };
  return { v, pub, saves, waves, keepers: () => saves.at(-1)?.keepers ?? [] };
}

test('a keep made before a page is published hears it published', async t => {
  const { v, pub, waves, keepers } = await serving(t);
  assert.equal(waves(NEC, () => load(v, 'later', 'keep', NEC)), 1, 'the first wave');
  assert.deepEqual(keepers().map(k => k.ship), ['~nec']);
  assert.equal(waves(NEC, () => pub.put('later', '# now')), 1, 'the publish reaches it');
  assert.deepEqual(v.trapped(), []);
});

test('a keep made again after a removal hears the page put back', async t => {
  const { v, pub, waves } = await serving(t);
  pub.put('page', '# one');
  load(v, 'page', 'keep', NEC);
  assert.equal(waves(NEC, () => pub.remove('page')), 1, 'the removal reaches it');
  //  the reader reloads: it drops and keeps the removed page again
  load(v, 'page', 'drop', NEC);
  load(v, 'page', 'keep', NEC);
  assert.equal(waves(NEC, () => pub.put('page', '# two')), 1, 'the republish reaches it');
});

test('one ship holds at most 64 keeps, and the others still keep', async t => {
  const { v, keepers } = await serving(t);
  for (let i = 0; i < 300; i++) load(v, `squat-${i}`, 'keep', NEC);
  assert.equal(keepers().filter(k => k.ship === '~nec').length, 64);
  load(v, 'real', 'keep', BUD);
  assert.ok(keepers().some(k => k.ship === '~bud'));
  //  a keep held already isn't evicted: ~nec's first is still there
  assert.ok(keepers().some(k => k.ship === '~nec' && k.folder.endsWith('/squat-0')));
});

test('a repeated keep saves nothing', async t => {
  const { v, pub, saves, keepers } = await serving(t);
  pub.put('real', '# real');
  load(v, 'real', 'keep', NEC);
  const after = saves.length;
  for (let i = 0; i < 50; i++) load(v, 'real', 'keep', NEC);
  assert.equal(saves.length, after);
  assert.equal(keepers().length, 1);
});

test('the cap holds at 256, and a drop is the dropper\'s own', { timeout: 60_000 }, async t => {
  const { v, keepers } = await serving(t);
  //  five ships, 64 each: 256 are kept, the last ship's last 64 are not
  for (let s = 1n; s <= 5n; s++) for (let i = 0; i < 64; i++) load(v, `p-${s}-${i}`, 'keep', s);
  assert.equal(keepers().length, 256);
  assert.ok(!keepers().some(k => k.ship === '~per'));
  //  ~bud's drop of ~nec's keep changes nothing; ~nec's own frees it
  load(v, 'p-1-0', 'drop', BUD);
  assert.equal(keepers().length, 256);
  load(v, 'p-1-0', 'drop', NEC);
  assert.equal(keepers().length, 255);
  //  the slot is free for a ship under its share
  load(v, 'free', 'keep', 5n);
  assert.equal(keepers().length, 256);
  assert.ok(keepers().some(k => k.ship === '~per'));
});
