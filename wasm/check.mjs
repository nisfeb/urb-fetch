// Checks serving with no ship, in two parts:
//
// 1. What mesa.mjs builds for other ships to clam and hash, against Hoon
//    itself: vere's `urbit eval` computes each value, and mesa.wasm must
//    agree. +sham names content in a grubbery %snap, maps and sets are
//    clammed against their treap shape, and ids and dates are text and
//    atoms lattice parses.
// 2. Pages published by one engine (pub.mjs) and read by another
//    (mesa.mjs's reader) in this process, two fake galaxies on a loop.
//    mesa.mjs's reader checks no hashes; part 1 and a real ship do.
//
//   URBIT=/path/to/urbit node check.mjs

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { DESK_BASE, da, fakeKeyfile, open, renderAs, uv } from './mesa.mjs';
import { markOf, publish } from './pub.mjs';
import { actions } from './forum.mjs';

const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));

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

await Promise.all([zod, nec, bud, wes, sev].map(u => u.close()));
process.exit(bad || checks.includes(false) ? 1 : 0);
