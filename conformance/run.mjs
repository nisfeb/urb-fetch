// Runs the conformance vectors the JS implements: every case's output,
// or its error, must be the vector's, word for word (and a hostile
// vector's within its time). The extension's need its
// checkout at the commit they name; without one they are skipped, or
// fail with REQUIRE_ALL set, as CI sets it.
//
//   node conformance/run.mjs [--ext ../nisfeb-extension]

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { engine, evaluate, extLib } from './lib.mjs';

const { values: opt } = parseArgs({ options: { ext: { type: 'string' } } });
const here = new URL('.', import.meta.url).pathname;
const JS = ['deurb', 'render-as', 'nouns', 'hash', 'messages', 'decoders', 'hostile-nouns'];
const EXT = ['render-ext', 'render-ext-random', 'hostile-render'];
const urb = await engine();
let lib = null, bad = 0;
if (opt.ext) {
  if (!existsSync(`${opt.ext}/lib/markdown.js`)) throw new Error(`no nisfeb-extension at ${opt.ext}`);
  lib = await extLib(opt.ext);
}
for (const name of [...JS, ...EXT]) {
  const v = JSON.parse(readFileSync(`${here}${name}.json`, 'utf8'));
  if (EXT.includes(name)) {
    if (!lib) {
      console.log(`skip  ${name}: no --ext checkout`);
      if (process.env.REQUIRE_ALL) bad++;
      continue;
    }
    const at = execFileSync('git', ['-C', opt.ext, 'rev-parse', '--short=7', 'HEAD'], { encoding: 'utf8' }).trim();
    if (at !== v.extension) throw new Error(`${name} is of nisfeb-extension ${v.extension}, the checkout is at ${at}`);
  }
  let fails = 0;
  for (const [i, c] of v.cases.entries()) {
    const r = await evaluate(urb, c, lib);
    const ok = r.skip ? false
      : c.error !== undefined ? r.error === c.error
      : JSON.stringify(r.out) === JSON.stringify(c.out);
    const slow = v.quick && r.ms > v.quick;
    if (!ok || slow) {
      fails++;
      if (fails <= 5) {
        console.log(`  ${name} ${i} ${c.fn}(${JSON.stringify(c.in).slice(0, 100)})${slow ? ` took ${r.ms} ms` : ''}\n    want ${JSON.stringify(c.error ?? c.out)?.slice(0, 200)}\n    got  ${JSON.stringify(r.skip ?? r.error ?? r.out)?.slice(0, 200)}`);
      }
    }
  }
  bad += fails;
  console.log(`${fails ? 'FAIL' : 'ok  '}  ${name}: ${v.cases.length - fails}/${v.cases.length}`);
}
await urb.close();
process.exit(bad ? 1 : 0);
