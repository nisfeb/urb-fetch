// Replays traces into the JS engine and checks each step's outputs, byte
// for byte (trace.mjs). With no files, every trace in ../traces. With
// --label, a trace whose timers have no names gets them from the replay
// (trace.mjs's replay, label) and is written back.
//
//   node replay.mjs [--label] [FILE.jsonl ...]

import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { difference, fromLines, replay, toLines } from './trace.mjs';

const { values: opt, positionals } = parseArgs({ allowPositionals: true, options: { label: { type: 'boolean' } } });
const dir = new URL('../traces/', import.meta.url).pathname;
const files = positionals.length ? positionals
  : readdirSync(dir).filter(f => f.endsWith('.jsonl')).sort((a, b) => parseInt(a) - parseInt(b)).map(f => dir + f);
if (!files.length) {
  console.error('replay: no traces');
  process.exit(1);
}
const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
const sha = createHash('sha256').update(wasm).digest('hex');
let bad = 0;
for (const f of files) {
  const t = fromLines(readFileSync(f, 'utf8'));
  const name = f.split('/').pop();
  if (t.head.wasm && t.head.wasm !== sha) console.log(`note  ${name} was recorded with mesa.wasm ${t.head.wasm.slice(0, 12)}, this is ${sha.slice(0, 12)}`);
  const start = Date.now();
  const r = await replay(t, { wasm, label: opt.label });
  if (r.ok) {
    console.log(`ok    ${name}: ${r.steps} steps (${Date.now() - start} ms)`);
    if (opt.label) writeFileSync(f, toLines(r.trace));
  } else {
    bad++;
    console.log(`FAIL  ${name}: step ${r.at} of ${r.steps} (${t.steps[r.at].in.k}) differs\n  ${difference(r.want, r.got)}`);
  }
}
process.exit(bad ? 1 : 0);
