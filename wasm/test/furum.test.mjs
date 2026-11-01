// furum's actions, as furum.mjs pokes them into the host's inbox.sig.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { FURUM_BASE, furum } from '../furum.mjs';

//  an engine that only records its pokes
const recorder = () => {
  const pokes = [];
  return { pokes, urb: { nouns: {}, grub: { poke: (...a) => { pokes.push(a); return Promise.resolve({ ok: true }); } } } };
};

test('newComment: a parent of 0 is the first comment, not none', async () => {
  const { pokes, urb } = recorder();
  const b = furum(urb, '~zod');
  await b.newComment('b', 7, 'top');
  await b.newComment('b', 7, 'top', null);
  await b.newComment('b', 7, 'top', undefined);
  await b.newComment('b', 7, 'to the first comment', 0);
  await b.newComment('b', 7, 'to the second', 1);
  for (const p of pokes) assert.deepEqual(p.slice(0, 4), ['~zod', FURUM_BASE, 'inbox.sig', ['/furum', 'msg']]);
  assert.deepEqual(pokes.map(p => p[4]), [
    ['act', 'new-comment', 'b', 7, null, 'top'],
    ['act', 'new-comment', 'b', 7, null, 'top'],
    ['act', 'new-comment', 'b', 7, null, 'top'],
    ['act', 'new-comment', 'b', 7, [null, 0], 'to the first comment'],
    ['act', 'new-comment', 'b', 7, [null, 1], 'to the second'],
  ]);
});
