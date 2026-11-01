// Seeds each renderer fuzz target's corpus from the rendering vectors:
// every body and link in conformance/inputs, and every hostile body,
// its long runs cut to 2,000 characters.
//
//   node core/urb-render/fuzz/seed.mjs   (writes core/urb-render/fuzz/corpus/<target>/)

import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const here = new URL('.', import.meta.url).pathname;
const conf = `${here}../../../conformance/`;
const json = f => JSON.parse(readFileSync(conf + f, 'utf8'));
const text = a => (typeof a === 'string' ? a : a?.$repeat ? text(a.$repeat[0]).repeat(Math.min(a.$repeat[1], 2000))
  : a?.$cat ? a.$cat.map(text).join('') : '');
const seeds = new Set();
for (const f of ['inputs/render.json', 'inputs/random.json']) {
  const r = json(f);
  for (const k of ['bodies', 'lines', 'links']) for (const s of r[k] ?? []) seeds.add(s);
}
for (const c of json('hostile-render.json').cases) for (const a of c.in) seeds.add(text(a));
for (const t of ['ext', 'gateway', 'two']) {
  mkdirSync(`${here}corpus/${t}`, { recursive: true });
  for (const s of seeds) {
    const b = Buffer.from(s, 'utf8');
    writeFileSync(`${here}corpus/${t}/${createHash('sha1').update(b).digest('hex')}`, b);
  }
}
console.log(`${seeds.size} seeds for ext, gateway and two`);
