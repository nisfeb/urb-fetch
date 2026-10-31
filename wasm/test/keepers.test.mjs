// The keeps other ships' grubbery makes of our files: only a file with
// something in it is kept, so one ship can't take every slot with names
// we never published; a repeated keep saves nothing; the cap holds; and
// one ship's drop leaves another's keep.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DESK_BASE } from '../mesa.mjs';
import { publish } from '../pub.mjs';
import { victim } from './fleet.mjs';

const vault = `${DESK_BASE}/pub/vault`;
const NEC = 1n, BUD = 2n, WES = 3n;

// a %grubbery-load of the vault file of [page], from [from]
const load = (v, page, tag, from) =>
  v.deliver('grubbery-load', [[v.path('/keep/page'), [0, v.path(`${vault}/${page}`), 'gmi']], tag, null], from);

async function serving(t) {
  const v = await victim();
  t.after(() => v.f.close());
  const saves = [];
  const pub = publish(v.urb, { save: s => saves.push(JSON.parse(JSON.stringify(s))) });
  return { v, pub, saves, keepers: () => saves.at(-1)?.keepers ?? [] };
}

test('keeps of pages we never published take no slot', async t => {
  const { v, pub, saves, keepers } = await serving(t);
  pub.put('real', '# real');
  const before = saves.length;
  for (let i = 0; i < 300; i++) load(v, `squat-${i}`, 'keep', NEC);
  assert.equal(saves.length, before, 'a keep of nothing saves nothing');
  load(v, 'real', 'keep', BUD);
  assert.deepEqual(keepers().map(k => k.ship), ['~bud']);
  //  a page taken down isn't kept anew either; one kept before stays
  pub.remove('real');
  load(v, 'real', 'keep', WES);
  assert.deepEqual(keepers().map(k => k.ship), ['~bud']);
  assert.deepEqual(v.trapped(), []);
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
  const { v, pub, keepers } = await serving(t);
  for (let i = 0; i < 257; i++) pub.put(`p-${i}`, `# ${i}`);
  for (let i = 0; i < 257; i++) load(v, `p-${i}`, 'keep', NEC);
  assert.equal(keepers().length, 256);
  assert.ok(!keepers().some(k => k.folder.endsWith('/p-256')));
  //  ~bud's drop of ~nec's keep changes nothing; ~nec's own frees it
  load(v, 'p-0', 'drop', BUD);
  assert.equal(keepers().length, 256);
  load(v, 'p-0', 'drop', NEC);
  assert.equal(keepers().length, 255);
  load(v, 'p-256', 'keep', BUD);
  assert.equal(keepers().length, 256);
  assert.ok(keepers().some(k => k.ship === '~bud' && k.folder.endsWith('/p-256')));
});
