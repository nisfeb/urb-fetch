// relayTransport's cases, shared by wasm/mesa.mjs's (relay.test.mjs) and
// the shim's (shim/fail.mjs), which keep the same function:
//
//   - a relay nobody runs fails the start at once
//   - one that accepts TCP and never answers the WebSocket upgrade
//     fails the start after 10 s instead of hanging
//   - close() while the socket is down stops the reconnect: no socket
//     is opened after it
//   - close() while a socket is still connecting closes that socket
//
// Each case is [what, async (relayTransport) => detail]: it throws, or
// gives an error's text, when it fails.

import { createServer } from 'node:net';

const sleep = ms => new Promise(r => setTimeout(r, ms));

// a WebSocket that opens in a microtask unless closed first, and counts
// what was made
function fakeSockets() {
  const made = [];
  class Fake {
    static OPEN = 1;
    constructor(url) {
      this.url = url;
      this.readyState = 0;
      made.push(this);
      queueMicrotask(() => {
        if (this.readyState !== 0) return;
        this.readyState = 1;
        this.onopen?.();
      });
    }
    send() {}
    close() {
      if (this.readyState === 3) return;
      this.readyState = 3;
      queueMicrotask(() => this.onclose?.());
    }
  }
  return { made, Fake };
}

async function withFake(f) {
  const real = globalThis.WebSocket;
  const fake = fakeSockets();
  globalThis.WebSocket = fake.Fake;
  try {
    return await f(fake.made);
  } finally {
    globalThis.WebSocket = real;
  }
}

export const relayCases = [
  ['a relay nobody runs fails the start', async relayTransport => {
    const t = relayTransport('ws://127.0.0.1:1');
    const e = await t.start('~zod', () => {}).then(() => null, e => e);
    if (e?.message !== "can't reach the relay at ws://127.0.0.1:1") return `start gave ${e?.message}`;
  }],
  ['a relay that never answers the upgrade fails the start after 10 s', async relayTransport => {
    const held = [];
    const srv = createServer(c => held.push(c));
    await new Promise(r => srv.listen(0, '127.0.0.1', r));
    const url = `ws://127.0.0.1:${srv.address().port}`;
    const t = relayTransport(url), at = Date.now();
    try {
      const e = await t.start('~zod', () => {}).then(() => null, e => e);
      const took = Date.now() - at;
      if (e?.message !== `can't reach the relay at ${url}`) return `start gave ${e?.message}`;
      if (took < 9_000 || took >= 15_000) return `${took} ms`;
    } finally {
      t.close();
      for (const c of held) c.destroy();
      srv.close();
    }
  }],
  ['close() while the socket is down stops the reconnect', async relayTransport => withFake(async made => {
    const t = relayTransport('ws://relay');
    await t.start('~zod', () => {});
    made[0].close();          // the relay drops it: a reconnect is due in 2 s
    await sleep(10);
    t.close();
    await sleep(2_300);
    if (made.length !== 1) return `${made.length - 1} socket(s) opened after close()`;
  })],
  ['close() while a socket is connecting closes it', async relayTransport => withFake(async made => {
    const t = relayTransport('ws://relay');
    const start = t.start('~zod', () => {}).then(() => null, e => e);
    t.close();
    const e = await start;
    await sleep(2_300);
    if (made[0].readyState !== 3) return 'the connecting socket was left open';
    if (!e) return 'start resolved after close()';
    if (made.length !== 1) return `${made.length - 1} socket(s) opened after close()`;
  })],
];
