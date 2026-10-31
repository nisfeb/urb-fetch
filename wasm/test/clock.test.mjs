// With the clock and the entropy fixed (open's now and entropy), a
// session's outputs are fixed too: wire nonces, the ids of the snaps we
// answer, page ids, and so every datagram.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { publish } from '../pub.mjs';
import { fixed, fleet } from './fleet.mjs';

// one session: ~zod publishes, ~nec reads a page over grubbery and by keen
async function session(seed) {
  const f = fleet(), zc = fixed(), nc = fixed();
  if (seed) nc.entropy(seed);   // a different stream for the reader
  nc.draws.length = 0;
  const zsent = [], nsent = [];
  const zod = await f.engine('~zod', 0, { now: zc.now, entropy: zc.entropy, sent: zsent });
  const nec = await f.engine('~nec', 1, { now: nc.now, entropy: nc.entropy, sent: nsent });
  const pub = publish(zod, {});
  const page = pub.put('notes/intro', '# Intro');
  const r = await nec.read('urb://~zod/notes/intro');
  await f.close();
  return { page, r, zsent, nsent, zdraws: zc.draws, ndraws: nc.draws };
}

test('fixed hooks give the same session twice', async () => {
  const a = await session(), b = await session();
  assert.equal(a.r.body, '# Intro');
  assert.equal(a.r.how, 'keen');
  //  the page's id and time come from the hooks
  assert.equal(a.page.at, 1_761_600_000_000);
  assert.equal(a.page.id, b.page.id);
  assert.match(a.page.id, /^0v[0-9a-v.]+$/);
  //  the nonce at open (4 bytes), then a page id (16) on the publisher;
  //  the reader draws its nonce, and the publisher one snap id (16) for
  //  each peek it answers
  assert.deepEqual(a.ndraws, [4]);
  assert.deepEqual(a.zdraws.slice(0, 2), [4, 16]);
  assert.ok(a.zdraws.length > 2 && a.zdraws.slice(2).every(n => n === 16));
  assert.deepEqual(a.zdraws, b.zdraws);
  //  every datagram, byte for byte
  assert.ok(a.nsent.length > 0 && a.zsent.length > 0);
  assert.deepEqual(a.nsent, b.nsent);
  assert.deepEqual(a.zsent, b.zsent);
});

test("the reader's entropy reaches the wire", async () => {
  const a = await session(), c = await session(7);
  assert.equal(c.r.body, '# Intro');
  //  only the reader's nonce differs: its peek's wire, sent to the publisher
  assert.notDeepEqual(a.nsent, c.nsent);
});

test('with no hooks, today\'s clock and entropy', async () => {
  const f = fleet();
  const zod = await f.engine('~zod', 0);
  const before = Date.now();
  const p = publish(zod, {}).put('a-page', 'x');
  assert.ok(p.at >= before && p.at <= Date.now());
  assert.equal(zod.now, Date.now);
  assert.equal(zod.entropy(8).length, 8);
  await f.close();
});
