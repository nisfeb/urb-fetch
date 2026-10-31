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
