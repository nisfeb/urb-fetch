// Checks serving with no ship, in three parts:
//
// 1. What mesa.mjs builds for other ships to clam and hash, against Hoon
//    itself: vere's `urbit eval` computes each value, and mesa.wasm must
//    agree. +sham names content in a grubbery %snap, maps and sets are
//    clammed against their treap shape, and ids and dates are text and
//    atoms lattice parses.
// 2. Pages published by one engine (pub.mjs) and read by another
//    (mesa.mjs's reader) in this process, two fake galaxies on a loop.
//    mesa.mjs's reader checks no hashes; part 1 and a real ship do.
// 3. Every grubbery message the engines sent in part 2, and the ones
//    only another ship's grubbery brings about (a %keep and the waves
//    after it, a %want), clammed by vere against grubbery's own molds
//    (test/grubbery-molds.hoon, copied verbatim from grubbery b126c50) as
//    the receiving ship clams them, and hashed: a message agrees only if
//    it fits its mold unchanged.
//
//   URBIT=/path/to/urbit node check.mjs

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { DESK_BASE, da, fakeKeyfile, messages, open, path, renderAs, snapOf, uv } from './mesa.mjs';
import { jam } from './jam.mjs';
import { markOf, publish } from './pub.mjs';
import { actions } from './forum.mjs';

const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));

// Each engine's module, as it opens, with its poke export wrapped: every
// message it sends is kept as [mark, noun as Hoon, its sham], for part 3
const sent = [], mods = [];
const realInstantiate = WebAssembly.instantiate;
WebAssembly.instantiate = async (w, imports) => {
  const r = await realInstantiate(w, imports);
  const X = { ...(r.instance ?? r).exports };
  const poke = X.poke;
  X.poke = (lo, hi, bone, a, m, n) => {
    const mem = new Uint8Array(X.memory.buffer);
    let e = m;
    while (mem[e]) e++;
    const mark = new TextDecoder().decode(mem.subarray(m, e));
    if (mark.startsWith('grubbery-')) sent.push([mark, hoonNoun(X, n), shamOf(X, n), treeOf(X, n)]);
    return poke(lo, hi, bone, a, m, n);
  };
  const opening = X.open;
  X.open = (...a) => (mods.push({ X, host: imports.host }), opening(...a));   // an engine, not a helper
  const inst = { exports: X };
  return r.instance ? { module: r.module, instance: inst } : inst;
};
const atomBytes = (X, n) => {
  const len = X.met3(n), p = X.alloc(len || 1);
  try {
    X.atom_bytes(n, p, len);
    return new Uint8Array(X.memory.buffer).slice(p, p + len);
  } finally { X.free(p); }
};
const atomInt = (X, n) => atomBytes(X, n).reduceRight((v, b) => (v << 8n) | BigInt(b), 0n);
// a noun as a Hoon literal: atoms as @ux, cells as [a b]
function hoonNoun(X, n) {
  if (!X.is_cell(n)) {
    const v = atomInt(X, n);
    return v ? '0x' + v.toString(16).replace(/\B(?=([0-9a-f]{4})+$)/g, '.') : '0x0';
  }
  return `[${hoonNoun(X, X.head(n))} ${hoonNoun(X, X.tail(n))}]`;
}
// a noun as JS: an atom a BigInt, a cell [head, tail]
const treeOf = (X, n) => (X.is_cell(n) ? [treeOf(X, X.head(n)), treeOf(X, X.tail(n))] : atomInt(X, n));
const textOf = v => Buffer.from(v ? v.toString(16).padStart(2, '0').replace(/^(.(..)*)$/, '0$1').match(/../g).reverse().join('') : '', 'hex').toString();
function shamOf(X, n) {
  const out = X.alloc(16);
  try {
    X.sham(n, out);
    return new Uint8Array(X.memory.buffer).slice(out, out + 16).reduceRight((v, b) => (v << 8n) | BigInt(b), 0n);
  } finally { X.free(out); }
}

// datagrams between engines in this process: galaxy g is 127.0.0.1:31337+g
const ends = new Map();
const loop = g => ({
  start(ship, onPacket) { ends.set(g, onPacket); },
  send(lane, bytes) {
    const f = ends.get(lane.name ? lane.galaxy : lane.port - 31337);
    if (f) setTimeout(() => f(0x7f000001, 31337 + g, bytes));
  },
  close() { ends.delete(g); },
});
const engine = async (ship, g) => {
  const keyfile = await fakeKeyfile(wasm, ship);
  return open({
    wasm,
    transport: loop(g),
    store: { load: () => ({ keyfile, rows: null, fresh: true }), saveKey() {}, commit() {} },
    log: () => {},
    fake: true,
  });
};
const zod = await engine('~zod', 0), nec = await engine('~nec', 1);
const { sham } = zod.nouns;

// ------------------------------------------------------------ 1. Hoon

// [hoon, js]: a hoon atom (printed as @ux) or knot, and what mesa.mjs makes
const cases = [];
const cord = s => `'${s}'`;
// numbers as Hoon writes them, in dotted groups
const ud = n => String(n).replace(/\B(?=(\d{3})+$)/g, '.');
const ux = n => '0x' + n.toString(16).replace(/\B(?=([0-9a-f]{4})+$)/g, '.');
//  a thousand, so the one in 256 whose salted mix ends in a zero byte is met
for (let i = 0; i < 1000; i++) cases.push([`(sham ${cord(`page ${i}`)})`, sham(`page ${i}`)]);
cases.push([`(sham 0)`, sham(0)], [`(sham [1 2])`, sham([1, 2])], [`(sham ${cord('x'.repeat(5000))})`, sham('x'.repeat(5000))]);
for (const n of [1, 2, 3, 7, 60]) {
  const keys = Array.from({ length: n }, (_, i) => `/page-${i}`);
  cases.push([`(sham (malt \`(list [@t @ud])\`~[${keys.map((k, i) => `[${cord(k)} ${i}]`).join(' ')}]))`,
              sham(new Map(keys.map((k, i) => [k, i])))]);
  cases.push([`(sham (silt \`(list @)\`~[${keys.map((_, i) => ud(1000 + i * 7919)).join(' ')}]))`,
              sham(new Set(keys.map((_, i) => 1000 + i * 7919)))]);
  cases.push([`(sham (malt \`(list [path @ud])\`~[${keys.map((k, i) => `[/pub${k}/gmi ${i}]`).join(' ')}]))`,
              sham(new Map(keys.map((k, i) => [['pub', k.slice(1), 'gmi', null], i])))]);
}
//  +revs-json's $json, as pub.mjs builds it
const row = (rev, id) => ['o', new Map([['rev', ['n', String(rev)]], ['id', ['s', id]]])];
const enjs = (rev, id) => `(pairs:enjs:format ~[['rev' (numb:enjs:format ${rev})] ['id' s+'${id}']])`;
cases.push([`(sham (pairs:enjs:format ~[['/a' ${enjs(3, '0v1')}] ['/b/c' ${enjs(12, '0v2')}]]))`,
            sham(['o', new Map([['/a', row(3, '0v1')], ['/b/c', row(12, '0v2')]])])]);
for (const ms of [0, 1761600000123, Date.UTC(2026, 9, 28, 12, 0, 0, 999)]) {
  cases.push([`(from-unix-ms:chrono:userlib ${ud(ms)})`, da(ms)]);
}
for (const n of [0n, 1n, 31n, 32n, 0xdeadbeefn, 2n ** 128n - 1n, 0x0123456789abcdef0123456789abcdefn]) {
  cases.push([`(scot %uv ${ux(n)})`, uv(n)]);
}

//  jam.mjs's +jam, which the conformance vectors write nouns in, against
//  vere's: atoms, cells, repeats (backrefs), and nouns the engine builds
const jtree = t => (Array.isArray(t) ? `[${jtree(t[0])} ${jtree(t[1])}]` : t ? ux(t) : '0x0');
const jamInt = t => jam(t).reduceRight((v, b) => (v << 8n) | BigInt(b), 0n);
const rep = [1n, 2n];
for (const t of [0n, 1n, 2n, 19n, 2n ** 64n, 2n ** 300n - 1n, [1n, 2n], [0n, 0n], [rep, rep], [[rep, rep], [rep, 3n]],
  [5n, [5n, [5n, 5n]]], [2n ** 70n, [2n ** 70n, 1n]],
  zod.nouns.tree(new Map([['a', 1], ['b', [2, 'x']], ['c', null]])), zod.nouns.tree(new Set([1, 2, 3, 4, 5, 6, 7])),
  zod.nouns.tree(messages.peek('/urb-fetch/0a0b0c0d/1', `${DESK_BASE}/pub`, 'revs')),
  zod.nouns.tree(messages.snap(['w', null], [0, path('/a/b'), 'c'], 2n ** 127n, ...snapOf({ mark: ['/lattice', 'page'], value: 'x'.repeat(300), rev: 3, at: 1761600000123 }, sham)))]) {
  cases.push([`(jam ${jtree(t)})`, jamInt(t)]);
}

//  every forum action forum.mjs sends, clammed as the host clams it
//  (+apply-forum-notice: ;;(action:ft q.q.sage)) against lattice's own
//  mold, test/lattice-forum-types.hoon, copied verbatim from release 81
//  (7584d6a). ;; fails a noun the clam would change, so a case agrees
//  only if the action fits and hashes the same.
const hoonOf = x => {   // a JS value as mesa.mjs makes its noun, as Hoon
  if (x === null) return '0x0';
  if (typeof x === 'string') x = [...new TextEncoder().encode(x)].reduceRight((v, b) => (v << 8n) | BigInt(b), 0n);
  if (typeof x === 'number' || typeof x === 'bigint') return BigInt(x) ? ux(BigInt(x)) : '0x0';
  return `[${x.map(hoonOf).join(' ')}]`;
};
const clam = x => `(sham ;;(action:ft \`*\`${hoonOf(x)}))`;
for (const a of [
  actions.newPost('test', { title: 'Hello' }),
  actions.newPost('test', { title: 'Hello', url: 'https://example.com', body: 'A body\nof two lines' }),
  actions.newPost('my-board-2', { title: 'ünïcode ✓', body: 'x'.repeat(2000) }),
  actions.newComment('test', 1, 'a comment'),
  actions.newComment('test', 1_000_000, 'a reply', 12),
  actions.upvote('test', { post: 3 }),
  actions.upvote('test', { post: 3, comment: 4 }),
  actions.downvote('test', { post: 3 }),
  actions.unvote('test', { post: 3, comment: 0 }),
]) cases.push([clam(a), sham(a)]);
//  and the clam bites: shapes no action has are refused
for (const bad of [['new-post', 'test', ['a', 'b'], null, null], ['new-post', 'test', 'x', [1, 'u'], null],
  ['upvote', 'test', ['reply', 1]], ['create', 'test'], ['remove-vote', 'test', ['comment', 1]]]) {
  cases.push([`?~((mole |.(;;(action:ft \`*\`${hoonOf(bad)}))) 1 0)`, 1n]);
}

const urbit = process.env.URBIT || 'urbit';
const types = readFileSync(new URL('test/lattice-forum-types.hoon', import.meta.url), 'utf8');
const expr = `=/  ft\n${types}\n~[${cases.map(([h, j]) => (typeof j === 'string' ? h : `\`@ux\`${h}`)).join(' ')}]`;
// the answer alone is on stdout
const out = execFileSync(urbit, ['eval'], { input: expr, encoding: 'utf8', maxBuffer: 1 << 26, stdio: ['pipe', 'pipe', 'ignore'] })
  .replace(/\x1b\[[0-9;]*m/g, '');
const got = out.match(/0x[0-9a-f.]+|~\.[^\s\]]+/g) || [];
if (got.length !== cases.length) console.error(out.slice(-600));
let bad = 0;
cases.forEach(([h, j], i) => {
  const want = typeof j === 'string' ? `~.${j}` : `0x${j.toString(16)}`;
  const have = typeof j === 'string' ? got[i] : got[i] && `0x${got[i].slice(2).replaceAll('.', '')}`;
  if (have !== want) {
    bad++;
    if (bad <= 10) console.error(`MISMATCH ${h.slice(0, 80)}\n  hoon ${have}\n  js   ${want}`);
  }
});
console.log(`${cases.length - bad}/${cases.length} agree with Hoon${got.length !== cases.length ? ` (read ${got.length} answers)` : ''}`);

// ------------------------------------------------------------ 2. a loop

const checks = [];
const expect = (what, ok) => {
  checks.push(ok);
  console.log(`${ok ? 'ok' : 'FAIL'}  ${what}`);
};
let saved = null;
const pub = publish(zod, { save: s => { saved = s; } });
pub.put('index', '# home');
pub.put('notes/intro', '# Intro\n=> /notes/more more');
expect('a one-letter name is refused', pub.put('n/x', 'no') === null);
const t = Date.now();
let r = await nec.read('urb://~zod/notes/intro');
expect(`a page reads back by keen (${Date.now() - t} ms)`, r.body === '# Intro\n=> /notes/more more' && r.how === 'keen');
r = await nec.read('urb://~zod/');
expect('the home page reads back', r.body === '# home');
const index = await nec.grub.peek('~zod', `${DESK_BASE}/pub`, 'index',
  n => nec.nouns.entries(n).map(e => '/' + [...nec.nouns.list(e.head)].map(s => s.text()).join('/')).sort());
expect('the index lists both', JSON.stringify(index) === '["/pub/index/gmi","/pub/notes/intro/gmi"]');
pub.put('notes/intro', '# Intro, again');
r = await nec.read('urb://~zod/notes/intro');
expect('an edit: a reader that read lately still sees the old one, as lattice', r.body === '# Intro\n=> /notes/more more');
const bud = await engine('~bud', 2);
r = await bud.read('urb://~zod/notes/intro');
expect(`an edit: a new reader reads it by ${r.how}`, r.body === '# Intro, again' && r.how === 'keen');
//  lattice 74 and later publish a page under its format's mark; before,
//  every page was %gmi, sniffed for HTML. Both read and render as lattice.
const formats = [
  ['fmt/md', 'md', '# Heading\n<b>raw</b>', 'md'],
  ['fmt/html', 'html', 'no tag first, still html', 'html'],
  ['fmt/text', 'text', '<b>looks like markup</b>', 'text'],
  ['fmt/css', 'css', 'body{color:red}', 'text'],
  ['fmt/js', 'js', 'alert(1)', 'text'],
  ['fmt/gmi', 'gmi', '# gemtext\n=> /a a', 'gmi'],
  ['fmt/old-html', 'gmi', '  <!doctype html><p>a pre-74 html page', 'html'],
];
for (const [name, mark, body] of formats) pub.put(name, body, mark);
expect('a mark lattice does not publish is refused', pub.put('fmt/x', 'x', 'del') === null);
const sev = await engine('~sev', 4);   // a reader whose list has them
for (const [name, mark, body, as] of formats) {
  r = await sev.read(`urb://~zod/${name}`);
  expect(`%${mark} reads back by keen as %${r.mark}, renders as ${renderAs(r.mark, r.body)}`,
    r.body === body && r.how === 'keen' && r.mark === mark && renderAs(r.mark, r.body) === as);
}
r = await nec.read('urb://~zod/fmt/md');   // nec's list is from before it
expect(`a page not on the reader's list reads by ${r.how} as %${r.mark}, as lattice's slow path`,
  r.how === 'peek' && r.mark === 'gmi' && r.body === '# Heading\n<b>raw</b>');
const vault = await sev.grub.peek('~zod', `${DESK_BASE}/pub/vault/fmt/md`, 'gmi', sev.nouns.cord);
expect('the vault file is %gmi whatever the format, as lattice keeps it', vault === '# Heading\n<b>raw</b>');
const as = [[undefined, '# x'], [undefined, '<p>'], ['gmi', '# x'], ['gmi', '\n <p>'], ['html', 'x'], ['md', '<p>'], ['text', '<p>'], ['css', '<p>'], ['noun', 'x']]
  .map(([m, b]) => renderAs(m, b)).join(' ');
expect(`renderAs: ${as}`, as === 'gmi html gmi html html md text text text');
const marks = ['a.MD', 'b.Html', 'c.htm', 'd.TXT', 'e.js', 'f.CSS', 'g.gmi', 'h', 'dir.md/i'].map(markOf).join(' ');
expect(`markOf, in any case: ${marks}`, marks === 'md html html text js css gmi gmi gmi');
pub.remove('notes/intro');
const wes = await engine('~wes', 3);
r = await wes.read('urb://~zod/notes/intro').catch(e => e);
expect('a removed page is gone', r instanceof Error && /not published/.test(r.message));
expect('save had pages and keeps', saved.pages['/notes/intro'].rev === 3 && saved.pages['/notes/intro'].body === null && Array.isArray(saved.keepers));


// ------------------------------------------------------------ 3. molds

//  the messages only another ship's grubbery brings about, by its hooks
//  (each engine's module in the order they opened: zod, then nec)
const [zm, nm] = mods;
const make = (X, x) => {   // a noun in X of a JS value: null, a string, a number, [a b ...]
  if (Array.isArray(x)) return x.slice(0, -1).reduceRight((t, h) => X.cell(make(X, h), t), make(X, x.at(-1)));
  const v = typeof x === 'string' ? [...new TextEncoder().encode(x)].reduceRight((v, b) => (v << 8n) | BigInt(b), 0n) : BigInt(x ?? 0);
  const b = [];
  for (let y = v; y > 0n; y >>= 8n) b.push(Number(y & 0xffn));
  const p = X.alloc(b.length || 1);
  new Uint8Array(X.memory.buffer).set(b, p);
  try { return X.atom(p, b.length); } finally { X.free(p); }
};
const hand = ({ X, host }, from, mark, value) => {   // libmesa hands the engine a poke
  const agent = new TextEncoder().encode('grubbery\0'), p = X.alloc(agent.length);
  new Uint8Array(X.memory.buffer).set(agent, p);
  const m = make(X, mark), n = make(X, value);
  try { host.on_poke(from, 0n, 0n, p, m, n); } finally { X.free(p); X.unref(m); X.unref(n); }
};
const hpath = s => [...s.split('/').filter(Boolean), null];
const kept = `${DESK_BASE}/pub/vault/fmt/md`;
//  ~nec keeps one of ~zod's pages: the first wave answers it, an edit
//  sends the next, and a drop ends it
const before = sent.length;
hand(zm, 1n, 'grubbery-load', [[hpath('/keep/page'), [0, hpath(kept), 'gmi']], 'keep', null]);
pub.put('fmt/md', '# Heading, edited', 'md');
hand(zm, 1n, 'grubbery-load', [[hpath('/keep/page'), [0, hpath(kept), 'gmi']], 'drop', null]);
expect('a keep is answered with a wave, and an edit sends another', sent.slice(before).filter(([m]) => m === 'grubbery-intake').length === 2);
//  a deep peek, of a directory ~zod doesn't serve, and a poke to its grub
await nec.grub.peek('~zod', `${DESK_BASE}/pub`, null, f => f.length);
await nec.grub.poke('~zod', `${DESK_BASE}`, 'forum.sig', ['/lattice', 'forum-action'], actions.upvote('test', { post: 1 }));
//  a %snap with no data, too big to inline, for a peek of ~nec's: it
//  asks for the content with a %want
const asked = nec.grub.peek('~zod', `${DESK_BASE}/pub/vault/nothing`, 'gmi', () => 1, 1000);
asked.catch(() => {});
const wsegs = [];
for (let w = sent.at(-1)[3][0][0]; Array.isArray(w); w = w[1]) wsegs.push(textOf(w[0]));
const jobe = 0x1234n;
hand(nm, 0n, 'grubbery-transfer', [hpath('/' + wsegs.join('/')), 'snap', [0, hpath(`${DESK_BASE}/pub/vault/nothing`), 'gmi'], 2n ** 500n + 7n,
  [null, [[1, 0x8000000d00000000n << 64n], ['firm', null, jobe], [[[jobe, null, null], null]]]], null]);
const tagOf = ([m, , , t]) => (m === 'grubbery-intake' ? 'wave' : textOf(t[1][0]));
expect(`a %snap with no data, for ${wsegs.join('/')}, is answered with a %want`, tagOf(sent.at(-1)) === 'want');

const molds = { 'grubbery-load': 'load:remo:nexus', 'grubbery-transfer': 'transfer:remo:nexus', 'grubbery-intake': 'intake:remo:nexus' };
const kinds = new Map();
for (const m of sent) if (!kinds.has(`${m[0]} ${m[1]}`)) kinds.set(`${m[0]} ${m[1]}`, m);
const mcases = [...kinds.values()];
//  and the clam bites: a load whose deep flag is 2, and a %snap whose
//  pace is no pace, are refused
const hoonTree = t => (Array.isArray(t) ? `[${hoonTree(t[0])} ${hoonTree(t[1])}]` : t ? ux(t) : '0x0');
const tas = x => BigInt('0x' + (Buffer.from(x).reverse().toString('hex') || '0'));
const peekT = mcases.find(m => tagOf(m) === 'peek')[3], snapT = mcases.find(m => tagOf(m) === 'snap' && Array.isArray(m[3][1][1][1][1]))[3];
const refused = [
  ['grubbery-load', hoonTree([peekT[0], [peekT[1][0], [peekT[1][1][0], 2n]]])],
  ['grubbery-transfer', hoonTree([snapT[0], [snapT[1][0], [snapT[1][1][0], [snapT[1][1][1][0], [[0n, [snapT[1][1][1][1][0][1][0], [[tas('fir'), snapT[1][1][1][1][0][1][1][0][1]], snapT[1][1][1][1][0][1][1][1]]]], snapT[1][1][1][1][1]]]]]])],
];
const grub = readFileSync(new URL('test/grubbery-molds.hoon', import.meta.url), 'utf8');
const mexpr = `${grub}\n~[${[...mcases, ...refused].map(([m, h]) => `\`@ux\`(fall (mole |.((sham ;;(${molds[m]} \`*\`${h})))) 0)`).join(' ')}]`;
const mout = execFileSync(urbit, ['eval'], { input: mexpr, encoding: 'utf8', maxBuffer: 1 << 26, stdio: ['pipe', 'pipe', 'ignore'] })
  .replace(/\x1b\[[0-9;]*m/g, '');
const mgot = mout.match(/0x[0-9a-f.]+/g) || [];
let mbad = 0;
const tags = new Map();
mcases.forEach(([m, h, sh], i) => {
  const have = mgot[i] && BigInt(mgot[i].replaceAll('.', ''));
  if (have !== sh) {
    mbad++;
    if (mbad <= 5) console.error(`MISMATCH ${m} ${h.slice(0, 160)}\n  hoon ${mgot[i]}\n  js   0x${sh.toString(16)}`);
  }
  const k = `${m.slice(9)} %${tagOf(mcases[i])}`;
  tags.set(k, (tags.get(k) ?? 0) + 1);
});
if (mgot.length !== mcases.length + refused.length) console.error(mout.slice(-600));
expect('a load with a deep flag of 2, and a %snap with no pace, are refused', mgot.slice(mcases.length).every(g => g === '0x0') && mgot.length === mcases.length + refused.length);
expect(`${mcases.length - mbad}/${mcases.length} grubbery messages fit grubbery's molds (${[...tags].map(([m, k]) => `${k} ${m}`).join(', ')})`, !mbad && mgot.length === mcases.length + refused.length);

await Promise.all([zod, nec, bud, wes, sev].map(u => u.close()));
process.exit(bad || checks.includes(false) ? 1 : 0);
