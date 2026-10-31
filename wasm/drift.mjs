// Compares two sets of traces by their digests (trace.mjs's digest): the
// committed ones and a fresh recording. Exit 1 if any drifted.
//
//   node drift.mjs COMMITTED_DIR FRESH_DIR

import { readdirSync, readFileSync } from 'node:fs';
import { digest, fromLines } from './trace.mjs';

const [a, b] = process.argv.slice(2);
const names = d => readdirSync(d).filter(f => f.endsWith('.jsonl')).sort();
let drift = 0;
for (const f of new Set([...names(a), ...names(b)])) {
  let x, y;
  try {
    x = digest(fromLines(readFileSync(`${a}/${f}`, 'utf8')));
    y = digest(fromLines(readFileSync(`${b}/${f}`, 'utf8')));
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
