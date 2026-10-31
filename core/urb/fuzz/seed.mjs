// Seeds each fuzz target's corpus from the conformance vectors: every
// noun in them, as jam (an input of mode 0, as src/lib.rs reads it),
// and for file_of and ball_of each fileOf and ballOf case's two nouns.
// hooks gets each noun twice, with each of its picks (the last byte);
// de_urb the urls deurb.json is made from.
//
//   node core/urb/fuzz/seed.mjs   (writes core/urb/fuzz/corpus/<target>/)

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const here = new URL('.', import.meta.url).pathname;
const vec = name => JSON.parse(readFileSync(`${here}../../../conformance/${name}.json`, 'utf8')).cases;
const ONE = ['cue', 'measure', 'list', 'entries', 'segments', 'json', 'tang', 'tang_text', 'ship'];
const nouns = new Set(), pairs = new Set();
const walk = x => {
  if (x && typeof x === 'object') {
    if (typeof x.$noun === 'string') nouns.add(x.$noun);
    else Object.values(x).forEach(walk);
  }
};
for (const name of ['decoders', 'hostile-nouns', 'nouns', 'messages']) {
  for (const c of vec(name)) {
    walk(c.in);
    if (typeof c.out === 'string' && /^[0-9a-f]+$/.test(c.out) && ['noun', 'peek', 'poke', 'snap', 'snapFile'].includes(c.fn)) nouns.add(c.out);
    if (['fileOf', 'ballOf'].includes(c.fn)) pairs.add(`${c.fn} ${c.in[0].$noun} ${c.in[1].$noun}`);
  }
}
const put = (target, i, bytes) => {
  mkdirSync(`${here}corpus/${target}`, { recursive: true });
  writeFileSync(`${here}corpus/${target}/seed-${i}`, bytes);
};
[...nouns].forEach((h, i) => {
  const jam = Buffer.from(h, 'hex');
  for (const t of ONE) put(t, i, t === 'cue' ? jam : Buffer.concat([Buffer.from([0]), jam]));
});
[...pairs].forEach((p, i) => {
  const [fn, a, b] = p.split(' ');
  const [x, y] = [Buffer.from(a, 'hex'), Buffer.from(b, 'hex')];
  const len = Buffer.from([x.length & 0xff, x.length >> 8]);
  const input = Buffer.concat([Buffer.from([0]), len, x, y]);
  put('file_of', `${fn}-${i}`, input);
  put('ball_of', `${fn}-${i}`, input);
});
[...nouns].forEach((h, i) => {
  const x = Buffer.from(h, 'hex');
  const len = Buffer.from([x.length & 0xff, x.length >> 8]);
  for (let pick = 0; pick < 7; pick++) put('hooks', `${i}-${pick}`, Buffer.concat([Buffer.from([0]), len, x, x, Buffer.from([pick])]));
});
JSON.parse(readFileSync(`${here}../../../conformance/inputs/urls.json`, 'utf8')).forEach((u, i) => put('de_urb', i, Buffer.from(u)));
console.log(`seeded ${nouns.size} nouns and ${pairs.size} pairs`);
