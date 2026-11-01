// relayTransport's start against relays that don't work: one nobody runs
// fails at once, and one that accepts TCP and never answers the
// WebSocket upgrade fails after 10 s instead of hanging.

import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { test } from 'node:test';
import { relayTransport } from '../mesa.mjs';

test('a relay nobody runs fails the start', async () => {
  const t = relayTransport('ws://127.0.0.1:1');
  await assert.rejects(t.start('~zod', () => {}), { message: "can't reach the relay at ws://127.0.0.1:1" });
});

test('a relay that never answers the upgrade fails the start after 10 s', async () => {
  const held = [];
  const srv = createServer(c => held.push(c));
  await new Promise(r => srv.listen(0, '127.0.0.1', r));
  const url = `ws://127.0.0.1:${srv.address().port}`;
  const t = relayTransport(url), at = Date.now();
  try {
    await assert.rejects(t.start('~zod', () => {}), { message: `can't reach the relay at ${url}` });
    const took = Date.now() - at;
    assert.ok(took >= 9_000 && took < 15_000, `${took} ms`);
  } finally {
    t.close();
    for (const c of held) c.destroy();
    srv.close();
  }
});
