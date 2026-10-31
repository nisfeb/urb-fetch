// Big, legitimate data passes the caps: a large page, a revision list
// near the cell cap, a big snap, and full furum buckets. Only a revision
// list over the cap is refused, and then read as an empty one, as
// lattice reads it.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DESK_BASE, MAX_CELLS } from '../mesa.mjs';
import { FURUM_BASE, furum } from '../furum.mjs';
import { publish } from '../pub.mjs';
import { fleet, late, victim } from './fleet.mjs';

test('a 2 MB page reads by keen', { timeout: 60_000 }, async () => {
  const f = fleet();
  const zod = await f.engine('~zod', 0), nec = await f.engine('~nec', 1);
  const body = '# big\n' + 'x'.repeat(2 << 20);
  publish(zod, {}).put('big', body);
  const r = await nec.read('urb://~zod/big');
  assert.equal(r.how, 'keen');
  assert.equal(r.body.length, body.length);
  await f.close();
});

// a $json object of n pages as a balanced treap of handles, built with
// the engine's own exports (its order isn't +gor's: the reader doesn't
// check it)
function revsNoun(v, n) {
  const row = i => ['o', [['id', ['s', `0v${i}`]], [['rev', ['n', '1']], null, null], null]];
  const tree = (lo, hi) => (lo >= hi ? null : (m => [[`/page-${m}`, row(m)], { n: v.build(tree(lo, m)) }, { n: v.build(tree(m + 1, hi)) }])(lo + hi >> 1));
  return ['o', { n: v.build(tree(0, n)) }];
}

// a snap of [value], inline, on the victim's wire k
const snapOf = (v, k, value) => [v.path(`/urb-fetch/${v.nonce}/${k}`), 'snap', null, 5,
  [null, [null, ['firm', null, 7], null]], [null, [[[9, null, value], null, null], [[7, null, 'leaf', 9, null], null, null]]]];

test('a revision list near the cap reads, and a big snap', { timeout: 60_000 }, async () => {
  const v = await victim();
  //  each page is 12 cells as a tree, as lattice's +revs-json makes it:
  //  16,600 of them is 199,200
  const p = v.urb.grub.peek('~nec', '/x', 'revs', v.urb.nouns.json);
  v.deliver('grubbery-transfer', snapOf(v, 1, revsNoun(v, 16_600)));
  const near = await late(p);
  assert.equal(Object.keys(near).length, 16_600);
  assert.deepEqual(near['/page-77'], { rev: 1, id: '0v77' });
  //  a big snap of another kind: a list of 90,000 atoms
  const q = v.urb.grub.peek('~nec', '/x', 'list', n => [...v.urb.nouns.list(n)].length);
  v.deliver('grubbery-transfer', snapOf(v, 2, [...Array(90_000).keys(), null]));
  assert.equal(await late(q), 90_000);
  await v.f.close();
});

test('a revision list over the cap still reads the page, by the vault', { timeout: 60_000 }, async () => {
  const f = fleet();
  const zod = await f.engine('~zod', 0), nec = await f.engine('~nec', 1);
  //  a $json array of MAX_CELLS numbers: over the cap once it is a snap
  const huge = ['a', [...Array(MAX_CELLS / 2).fill(['n', '1']), null]];
  zod.grub.serve((folder, name) => {
    if (folder === `${DESK_BASE}/pub` && name === 'revs') return { mark: ['/', 'json'], value: huge, rev: 1, at: 0 };
    if (folder === `${DESK_BASE}/pub/vault/page` && name === 'gmi') return { mark: ['/lattice', 'page'], value: '# by the vault', rev: 1, at: 0 };
    return null;
  });
  const r = await late(nec.read('urb://~zod/page'));
  assert.equal(r.body, '# by the vault');
  assert.equal(r.how, 'peek');
  await f.close();
});

test('a full furum bucket: 100 posts, and a thread of comments', { timeout: 60_000 }, async () => {
  const f = fleet();
  const host = await f.engine('~zod', 0), us = await f.engine('~nec', 1);
  const board = `${FURUM_BASE}/boards/b`;
  const da = 0x8000000cce9e0d80n << 64n;
  const posts = new Map(Array.from({ length: 100 }, (_, i) =>
    [i, [i, 1, `post ${i}`, null, [null, 'x'.repeat(2000)], da]]));
  const comments = new Map(Array.from({ length: 400 }, (_, i) =>
    [i, [i, i ? [null, i - 1] : null, 1, `comment ${i} `.repeat(20), da]]));
  const threads = new Map([[7, [400, comments]]]);
  const files = {
    [`${board}/pub conf`]: [1, 100, null],
    [`${board}/content/posts b0`]: [1, posts],
    [`${board}/content/threads b0`]: [1, threads],
  };
  host.grub.serve((folder, name) => {
    const value = files[`${folder} ${name}`];
    return value === undefined ? null : { mark: ['/', 'noun'], value, rev: 1, at: 0 };
  });
  const b = furum(us, '~zod');
  const got = await late(b.posts('b'));
  assert.equal(got.length, 100);
  assert.equal(got[0].id, 99);
  assert.equal(got[42].body.length, 2000);
  const cs = await late(b.comments('b', 7));
  assert.equal(cs.length, 400);
  assert.equal(cs[3].parent, 2);
  await f.close();
});
