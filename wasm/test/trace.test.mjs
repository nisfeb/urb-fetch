// The recorder's own check, on the in-process loop: a session recorded
// against a fake publisher replays into a fresh engine byte for byte,
// step by step, and a trace that was changed does not.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fakeKeyfile } from '../mesa.mjs';
import { publish } from '../pub.mjs';
import { difference, fromLines, recorder, replay, toLines } from '../trace.mjs';
import { fleet, wasm } from './fleet.mjs';

// ~nec, recorded, reads ~zod's pages; then publishes one of its own
async function session() {
  const f = fleet();
  const zod = await f.engine('~zod', 0);
  const pub = publish(zod, {});
  pub.put('notes/intro', '# Intro');
  pub.put('notes/big', 'x'.repeat(15_000), 'md');
  const rec = recorder({ wasm, transport: f.transport(1) });
  const keyfile = await fakeKeyfile(wasm, '~nec');
  assert.deepEqual(await rec.call('open', { keyfile, fresh: true }, { fake: true }), { our: '~nec' });
  const a = await rec.call('read', 'urb://~zod/notes/intro');
  assert.equal(a.body, '# Intro');
  const b = await rec.call('read', 'urb://~zod/notes/big');
  assert.equal(b.mark, 'md');
  await assert.rejects(rec.call('read', 'urb://~zod/nothing'), /not published/);
  await rec.call('pub');
  const p = await rec.call('pub.put', 'mine', 'hello', 'text');
  assert.match(p.id, /^0v/);
  await rec.call('close');
  await f.close();
  return rec.trace({ name: 'loop' });
}

test('a recorded session replays byte for byte', async () => {
  const t = fromLines(toLines(await session()));
  const kinds = new Set(t.steps.map(s => s.in.k));
  assert.deepEqual([...kinds].sort(), ['call', 'hear', 'tick']);
  assert.ok(t.steps.some(s => s.in.eny.length), 'it drew entropy');
  assert.ok(t.steps.some(s => s.out.some(o => o.send)), 'it sent');
  assert.ok(t.steps.some(s => s.out.some(o => o.commit)), 'it saved');
  //  every tick names its timer, and none comes before its due
  const ticks = t.steps.filter(s => s.in.k === 'tick');
  assert.ok(ticks.length && ticks.every(s => s.in.kind && s.in.due <= s.in.now && !s.in.early));
  assert.ok(ticks.some(s => s.in.kind.startsWith('point ')) && ticks.some(s => s.in.kind === 'close'));
  const r = await replay(t, { wasm });
  assert.ok(r.ok, r.ok ? '' : `step ${r.at}: ${difference(r.want, r.got)}`);
  assert.equal(r.steps, t.steps.length);
});

test('a changed trace does not replay', async () => {
  const t = fromLines(toLines(await session()));
  // one bit of one datagram heard
  const k = t.steps.findIndex(s => s.in.k === 'hear');
  const b = t.steps[k].in.bytes;
  t.steps[k].in.bytes = b.slice(0, -2) + ((parseInt(b.slice(-2), 16) ^ 1).toString(16).padStart(2, '0'));
  const r = await replay(t, { wasm, patience: 2000 });
  assert.equal(r.ok, false);
  assert.ok(r.at >= k);
  // one recorded output
  const u = fromLines(toLines(await session()));
  const j = u.steps.findIndex(s => s.out.some(o => o.send));
  u.steps[j].out.find(o => o.send).bytes += '00';
  const q = await replay(u, { wasm, patience: 2000 });
  assert.equal(q.ok, false);
  assert.equal(q.at, j);
  // a tick that names another timer
  const v = fromLines(toLines(await session()));
  const n = v.steps.findIndex(s => s.in.k === 'tick');
  v.steps[n].in.kind = 'peek /nothing';
  const x = await replay(v, { wasm, patience: 2000 });
  assert.equal(x.ok, false);
  assert.equal(x.at, n);
});
