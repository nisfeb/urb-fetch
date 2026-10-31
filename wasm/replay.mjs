// Replays traces into the JS engine and checks each step's outputs, byte
// for byte (trace.mjs). With no files, every trace in ../traces.
//
//   node replay.mjs [FILE.jsonl ...]

import { readdirSync, readFileSync } from 'node:fs';
import { difference, fromLines, replay } from './trace.mjs';

const dir = new URL('../traces/', import.meta.url).pathname;
const files = process.argv.length > 2 ? process.argv.slice(2)
  : readdirSync(dir).filter(f => f.endsWith('.jsonl')).sort((a, b) => parseInt(a) - parseInt(b)).map(f => dir + f);
if (!files.length) {
  console.error('replay: no traces');
  process.exit(1);
}
const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
let bad = 0;
for (const f of files) {
  const t = fromLines(readFileSync(f, 'utf8'));
  const start = Date.now();
  const r = await replay(t, { wasm });
  const name = f.split('/').pop();
  if (r.ok) console.log(`ok    ${name}: ${r.steps} steps (${Date.now() - start} ms)`);
  else {
    bad++;
    console.log(`FAIL  ${name}: step ${r.at} of ${r.steps} (${t.steps[r.at].in.k}) differs\n  ${difference(r.want, r.got)}`);
  }
}
process.exit(bad ? 1 : 0);
