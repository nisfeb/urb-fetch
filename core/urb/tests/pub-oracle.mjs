// The JS as the oracle for meta.pub (publish.rs's tests): reads
// {cases: [{state, now, ops}]} as JSON on stdin, and for each case opens
// a fake ~zod with a fixed clock and counted entropy, publishes from the
// saved state, and runs the ops (['put', name, body, mark?], ['remove',
// name] or ['pages']), each a second after the last. It writes, for
// each op, its result and every save it made, as JSON.stringify writes
// them: [[{result, saves}]].
//
//   node core/urb/tests/pub-oracle.mjs < in.json

import { readFileSync } from 'node:fs';
import { fakeKeyfile, open } from '../../../wasm/mesa.mjs';
import { publish } from '../../../wasm/pub.mjs';

const wasm = readFileSync(new URL('../../../wasm/build/mesa.wasm', import.meta.url));
const input = JSON.parse(readFileSync(0, 'utf8'));
const keyfile = await fakeKeyfile(wasm, '~zod');
const out = [];
for (const c of input.cases) {
  let n = 0, clock = c.now;
  const urb = await open({
    wasm, fake: true, log: () => {},
    transport: { start() {}, send() {}, close() {} },
    store: { load: () => ({ keyfile, rows: null, fresh: true }), saveKey() {}, commit() {} },
    now: () => clock,
    entropy: k => Uint8Array.from({ length: k }, () => n++ & 0xff),
    timers: { setTimeout: () => 0, clearTimeout() {} },
  });
  const saves = [];
  const pub = publish(urb, { ...c.state, save: s => saves.push(JSON.stringify(s)) });
  const steps = [];
  for (const [op, ...a] of c.ops) {
    clock += 1000;
    saves.length = 0;
    let result;
    try {
      result = JSON.stringify(op === 'put' ? pub.put(...a) : op === 'remove' ? pub.remove(...a) : pub.pages());
    } catch (e) {
      result = { error: e.message };
    }
    steps.push({ result, saves: [...saves] });
  }
  out.push(steps);
}
// the engines' timers never fire, so nothing ends them: exit once written
process.stdout.write(JSON.stringify(out), () => process.exit(0));
