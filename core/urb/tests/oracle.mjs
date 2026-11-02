// The JS as an oracle for urb's property tests (props.rs): reads
// {values, texts, numbers} as JSON on stdin and writes what mesa.mjs,
// a TextDecoder and Number() make of each:
//
//   values   each a value in trace.mjs's encoding: noun()'s jam as hex,
//            or {error} with its TypeError's text
//   texts    each bytes as hex: their text, as mesa.mjs's Noun reads it
//   numbers  each a string: Number() of it, NaN, the infinities and -0
//            as {"$num": ...}
//
//   node core/urb/tests/oracle.mjs < in.json

import { readFileSync } from 'node:fs';
import { engine } from '../../../conformance/lib.mjs';
import { hex, jam, unhex } from '../../../wasm/jam.mjs';
import { dec } from '../../../wasm/trace.mjs';

const input = JSON.parse(readFileSync(0, 'utf8'));
const urb = await engine();
const decoder = new TextDecoder();
const num = n => (Number.isNaN(n) ? { $num: 'NaN' } : !Number.isFinite(n) ? { $num: String(n) }
  : Object.is(n, -0) ? { $num: '-0' } : n);
const out = {
  values: input.values.map(v => {
    try {
      return hex(jam(urb.nouns.tree(dec(v))));
    } catch (e) {
      return { error: e.message };
    }
  }),
  texts: input.texts.map(h => decoder.decode(unhex(h))),
  numbers: input.numbers.map(s => num(Number(s))),
};
await urb.close();
process.stdout.write(JSON.stringify(out));
