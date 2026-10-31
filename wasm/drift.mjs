// Compares two sets of traces by their digests (trace.mjs's digest: the
// calls, results and events) and by what each put on the wire (wire:
// the kinds of message, keen and scry): the committed ones and a fresh
// recording. Exit 1 if any drifted.
//
//   node drift.mjs COMMITTED_DIR FRESH_DIR

import { readdirSync, readFileSync } from 'node:fs';
import { digest, fromLines, wire } from './trace.mjs';

const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));

const [a, b] = process.argv.slice(2);
const names = d => readdirSync(d).filter(f => f.endsWith('.jsonl')).sort();
let drift = 0;
for (const f of new Set([...names(a), ...names(b)])) {
  let x, y;
  try {
    const [s, t] = [a, b].map(d => fromLines(readFileSync(`${d}/${f}`, 'utf8')));
    x = [...digest(s), { wire: await wire(s, { wasm }) }];
    y = [...digest(t), { wire: await wire(t, { wasm }) }];
  } catch (e) {
    drift++;
    console.log(`DRIFT ${f}: ${e.message}`);
    continue;
  }
  const i = [...Array(Math.max(x.length, y.length)).keys()].find(i => JSON.stringify(x[i]) !== JSON.stringify(y[i]));
  if (i === undefined) console.log(`same  ${f}: ${x.length} calls`);
  else {
    drift++;
    console.log(`DRIFT ${f}: call ${i}\n  committed ${JSON.stringify(x[i])?.slice(0, 400)}\n  recorded  ${JSON.stringify(y[i])?.slice(0, 400)}`);
  }
}
process.exit(drift ? 1 : 0);
