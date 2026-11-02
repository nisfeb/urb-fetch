// Writes the conformance vectors the JS generates (docs/rust-core.md,
// "The conformance suite"; each file's "about" says what its cases
// are, and lib.mjs how a case runs): the engine's urls, nouns, hashes,
// messages and decoders from ../wasm, and with --ext DIR the extension's
// renderers from a nisfeb-extension checkout. Each case's output is what the code
// gives today. The hostile ones are written by hand in hostile.mjs, with
// the error each must give; this writes them out with their nouns
// jammed, and stops if the JS does not pass one.
//
//   node conformance/gen.mjs [--ext ../nisfeb-extension]
//
// The Go's are written by `UPDATE_VECTORS=1 go test -run TestConformance`.

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { forum } from '../wasm/forum.mjs';
import { furum, FURUM_BASE } from '../wasm/furum.mjs';
import { hex, jam } from '../wasm/jam.mjs';
import { DESK_BASE, da, path } from '../wasm/mesa.mjs';
import { enc } from '../wasm/trace.mjs';
import { engine, evaluate, extLib, fakeUrb, PAGE } from './lib.mjs';
import { hostileNouns, hostileRender, quick } from './hostile.mjs';
import { randomRender } from './random.mjs';

const { values: opt } = parseArgs({ options: { ext: { type: 'string' } } });
const here = new URL('.', import.meta.url).pathname;
const urb = await engine();
const { sham, tree } = urb.nouns;
const N = x => ({ $noun: hex(jam(tree(x))) });   // a value as noun() builds it, as a vector's noun
const C = (fn, ...a) => ({ fn, in: enc(a).map((v, i) => (a[i]?.$noun ? a[i] : v)) });

async function write(name, about, cases, more = {}, lib) {
  for (const c of cases) {
    const r = await evaluate(urb, c, lib);
    if (r.skip) throw new Error(`${name}: ${r.skip}`);
    if (r.error !== undefined) c.error = r.error;
    else c.out = r.out;
  }
  writeFileSync(`${here}${name}.json`, JSON.stringify({ about, ...more, cases }, null, 1) + '\n');
  console.log(`${name}.json: ${cases.length} cases`);
}

// ------------------------------------------------------------ urls

const urls = JSON.parse(readFileSync(`${here}inputs/urls.json`, 'utf8'));
await write('deurb', "wasm/mesa.mjs's deUrb(url): [ship, rel, mount], a post id exact ($big past 2^53 - 1), or the error",
  urls.map(u => C('deUrb', u)));

// ------------------------------------------------------------ which renderer

const bodies = ['# x', '<p>', '  <!doctype html>', '\n <p>', ' <b>', '﻿<x>', '\u0085<x>', '\v<x>', '\f<x>', '　<x>',
  '', 'x', `${' '.repeat(255)}<`, `${' '.repeat(256)}<`, `${'　'.repeat(255)}<`, `${'\u{1F600}'.repeat(128)}<`];
const marks = [undefined, null, 'gmi', 'html', 'md', 'text', 'css', 'js', 'noun', 'del', 'GMI'];   // not '': that is no mark to urb.go, a mark no page has to mesa.mjs
await write('render-as', "wasm/mesa.mjs's renderAs(mark, body) and looksHtml(body); urb.go's renderAs and looksHTML give the same (no mark is \"\" there)", [
  ...marks.flatMap(m => bodies.map(b => C('renderAs', m, b))),
  ...bodies.map(b => C('looksHtml', b)),
]);

// ------------------------------------------------------------ nouns and hashes

const big = 2n ** 64n, keyAtoms = [1n, 0x1234n, big, big + 1n, 2n ** 127n + 5n, sham('a'), sham('b'), sham([1, 2])];
const values = [
  null, true, false, 0, 1, 255, 256, 65535, 2 ** 53 - 1, big, 2n ** 128n + 1n, 2n ** 512n - 1n,
  '', 'a', 'cord', 'ünïcode ✓', 'x'.repeat(1000), [1, 2], [1, 2, 3], ['a', null], [[1, 2], [3, 4]], [null, null, null],
  path('/a/b/c'), path('/'), path('/g/x/1/grubbery//1/pub/page/notes/intro/0v1'), path(DESK_BASE),
  ...[0, 1, 2, 3, 5, 8, 13, 40].flatMap(n => [
    new Map(Array.from({ length: n }, (_, i) => [`key-${i}`, i])),
    new Map(Array.from({ length: n }, (_, i) => [path(`/pub/p${i}/gmi`), [i, `v${i}`]])),
    new Set(Array.from({ length: n }, (_, i) => 1000 + i * 7919)),
    new Set(Array.from({ length: n }, (_, i) => `s${i}`)),
  ]),
  new Map(keyAtoms.map((k, i) => [k, [0, `noun ${i}`]])),
  new Set(keyAtoms),
  new Map([['a', new Map([['b', new Set([1, 2])]])]]),
  ['o', new Map([['/a', ['o', new Map([['rev', ['n', '3']], ['id', ['s', '0v1']]])]]])],
];
await write('nouns', "noun(value) as mesa.mjs builds it, as jam hex: maps and sets as +put builds them (the value in trace.mjs's encoding: an array of two or more is cells, null is ~, true and false loobeans)",
  values.map(v => C('noun', v)));
const ms = [0, 1, 999, 1000, 1761600000123, Date.UTC(2026, 9, 28, 12, 0, 0, 999), 8.64e15];
await write('hash', "sham (hex) and mug of a value as noun() builds it, and mesa.mjs's uv and da; check.mjs checks each kind against vere", [
  ...values.map(v => C('sham', v)),
  ...values.map(v => C('mug', v)),
  ...[0n, 1n, 31n, 32n, 1023n, 1024n, 0xdeadbeefn, 2n ** 128n - 1n, 2n ** 512n - 3n].map(n => C('uv', n)),
  ...ms.map(m => C('da', m)),
]);

// ------------------------------------------------------------ messages

const file = { mark: ['/lattice', 'page'], value: '# A page\n=> /x x', rev: 3, at: 1761600000123 };
const index = { mark: ['/lattice', 'pub-index'], value: new Map([[path('/pub/index/gmi'), [da(1761600000123), 7, sham('# home')]]]), rev: 2, at: 1761600000000 };
const revs = { mark: ['/', 'json'], value: ['o', new Map([['/index', ['o', new Map([['rev', ['n', '1']], ['id', ['s', '0v1.abc']]])]]])], rev: 2, at: 1761600000000 };
const dest = [0, path(`${DESK_BASE}/pub/vault/index`), 'gmi'];
//  what furum.mjs and forum.mjs hand grub.poke, by their own code
const pokes = [];
const poking = fakeUrb(urb, { poke: async (...a) => (pokes.push(a), { ok: true }), peek: async () => true });
const fu = furum(poking, '~zod'), fo = forum(poking, '~zod');
await fu.newPost('board', { title: 'A title', url: 'https://example.com', body: 'A body' });
await fu.newPost('board', { title: 'No url or body' });
await fu.newComment('board', 7, 'a comment');
await fu.newComment('board', 7, 'a reply', 2);
await fu.upvote('board', 7);
await fo.newPost('m1btest', { title: 'Hello', url: '', body: 'x' });
await fo.newPost('m1btest', { title: 'ünïcode ✓', url: 'https://example.com/a', body: 'y'.repeat(3000) });
await fo.newComment('m1btest', '12', 'a comment');
await fo.newComment('m1btest', 2n ** 60n, 'a reply', 3);
await fo.upvote('m1btest', { post: 1 });
await fo.upvote('m1btest', { post: 1, comment: 0 });
await fo.downvote('m1btest', { post: 2 });
await fo.unvote('m1btest', { post: 2, comment: 9 });
await write('messages', "each message mesa.mjs sends grubbery, from its builders (mesa.mjs's messages; snapFile is a snap of snapOf(file), intakeOf an intake of waveOf(file, 'gmi')), as jam hex. The furum and forum pokes are what furum.mjs and forum.mjs hand grub.poke; forum.* are forum.mjs's actions", [
  C('peek', '/urb-fetch/0a0b0c0d/1', `${DESK_BASE}/pub`, 'revs'),
  C('peek', '/urb-fetch/0a0b0c0d/2', `${DESK_BASE}/forum/boards/m1btest/content`, null),
  C('peek', '/urb-fetch/ffffffff/99', '/', 'x'),
  C('pack', path('/grub-poke/0a0b0c0d/3')),
  C('want', path('/urb-fetch/0a0b0c0d/1'), dest, 2n ** 511n + 12345n),
  C('snap', path('/urb-fetch/0a0b0c0d/1'), dest, 2n ** 127n + 1n, null, null),
  ...[file, index, revs, { ...file, value: 'z'.repeat(9000) }].map(f => C('snapFile', path('/urb-fetch/0a0b0c0d/1'), dest, 0xfeedn, f)),
  C('intakeOf', path('/page'), dest, file),
  C('intakeOf', null, dest, { rev: 0, at: 0 }),
  C('intakeOf', path('/page'), dest, { rev: 4, at: 1761600009999 }),
  ...pokes.map(([, folder, name, mark, value]) => C('poke', '/grub-poke/0a0b0c0d/9', folder, name, mark, value)),
  C('forum.newPost', 'b', { title: 'T' }), C('forum.newPost', 'b', { title: 'T', url: 'u', body: 'b' }), C('forum.newPost', 'b', { title: '' }),
  C('forum.newComment', 'b', 1, 'c'), C('forum.newComment', 'b', 1, 'c', 0), C('forum.newComment', 'b', 1, ''),
  C('forum.newComment', 'b', -1, 'c'), C('forum.newComment', 'b', 2n ** 256n, 'c'), C('forum.newComment', 'b', '007', 'c'),
  C('forum.upvote', 'b', { post: 1 }), C('forum.upvote', 'b', { post: 1, comment: 2 }),
  C('forum.downvote', 'b', { post: 0 }), C('forum.unvote', 'b', { post: 5, comment: 0 }),
]);

// ------------------------------------------------------------ decoders

const tape = s => [...new TextEncoder().encode(s)].reduceRight((t, c) => [c, t], null);
const leaf = s => ['leaf', ...[...new TextEncoder().encode(s)], null];
const lobe = x => sham(x);
//  a snap and its silo for one file, as grubbery sends them
const fileSnap = (value, mark = ['/lattice', 'page']) => {
  const nobe = lobe(value), ject = ['leaf', nobe, [[path(mark[0]), mark[1]], 0, null], true, null], jobe = lobe(ject);
  return [[[3, da(NOWISH)], ['firm', null, jobe], [new Set([jobe]), new Set([nobe])]],
          [new Map([[nobe, [0, value]]]), new Map([[jobe, [0, ject]]])]];
};
const NOWISH = 1761600000123;
//  a directory's: [%tree nek gain bang fil dir], files and subdirectories
function dirSnap(files, dirs = {}) {
  const nouns = new Map(), jects = new Map();
  const put = (k, v, m) => { m.set(k, [0, v]); return k; };
  const build = (fs, ds) => {
    const fil = new Map(Object.entries(fs).map(([name, value]) => {
      const nobe = put(lobe(value), value, nouns);
      const ject = ['leaf', nobe, [[path('/'), 'noun'], 0, null], true, null];
      return [name, put(lobe(ject), ject, jects)];
    }));
    const dir = new Map(Object.entries(ds).map(([name, [f, d]]) => [name, [build(f, d), null]]));
    const t = ['tree', null, true, null, fil, dir];
    return put(lobe(t), t, jects);
  };
  const top = build(files, dirs);
  return [[[1, da(NOWISH)], ['firm', null, top], [new Set(jects.keys()), new Set(nouns.keys())]], [nouns, jects]];
}
const jsonN = ['o', new Map([['a', ['n', '1']], ['b', ['a', [['s', 'x'], [['b', true], null]]]], ['c', ['o', new Map()]]])];
const tang = ['err', leaf('a leaf'), ['rose', [tape(', '), [tape('['), tape(']')]], [leaf('x'), [leaf('y'), null]]],
  ['palm', [tape(' '), [tape('<'), [tape('|'), tape('>')]]], [leaf('p'), null]], null];
const bucket = entries => [1, new Map(entries)];
const post = (id, author, title, url, body) => [id, author, title, url ? [null, url] : null, body ? [null, body] : null, da(NOWISH + id)];
const comment = (id, parent, author, body) => [id, parent === null ? null : [null, parent], author, body, da(NOWISH + id)];
const FB = `${FURUM_BASE}/boards/news`;
const furumFiles = {
  [`${FB}/pub conf`]: N([1, 2, null]),
  [`${FB}/content/posts b0`]: N(bucket([[0, post(0, 0n, 'First', 'https://a.b', null)], [1, post(1, 1n, 'Second', null, 'body')]])),
  [`${FB}/content/threads b0`]: N(bucket([[0, [2, new Map([[0, comment(0, null, 1n, 'hi')], [1, comment(1, 0, 0n, 'reply')]])]]])),
  [`${FB}/content/votes b0`]: N(bucket([[['post', 0], [new Set([0n, 1n]), new Set()]], [['comment', [0, 1]], [new Set([1n]), new Set([0n])]]])),
};
const LB = `${DESK_BASE}/forum/boards/m1btest`;
const role = r => r;
const info = ['m1btest', 'M1b test board', 'a board', 0n, da(NOWISH), 'poster', true];
const forumDirs = (extra = {}) => ({
  [`${LB}/pub`]: [[['card'], [1, info]], [['roles'], [1, new Map([[1n, role('mod')]])]], [['conf'], [1, [3, [null, [5, 2n ** 64n * 3600n]]]]]],
  [`${LB}/content`]: [
    [['pins'], [1, new Set([2])]], [['sidebar'], [1, 'Be kind.']],
    [['posts', 'b0'], [1, new Map([[1, post(1, 0n, 'One', null, 'x')], [2, post(2, 1n, 'Two', 'https://c.d', null)]])]],
    [['threads', 'b0'], [1, new Map([[1, [1, new Map([[0, comment(0, null, 1n, 'first')]])]]])]],
    [['votes', 'b0'], [1, new Map([[['post', 1], [new Set([1n]), new Set()]], [['comment', [1, 0]], [new Set(), new Set([0n])]]])]],
    ...(extra.content ?? []),
  ],
});
const dirsN = d => Object.fromEntries(Object.entries(d).map(([k, files]) => [k, files.map(([p, v]) => [p, N(v)])]));
const [fs, fsilo] = fileSnap('# A page body');
const [ds, dsilo] = dirSnap({ card: 'c' }, { posts: [{ b0: 'p' }, {}], threads: [{}, { deep: [{ x: 'y' }, {}] }] });
const note = (from, to, mark, noun) => ({ from, to, mark, noun: N(noun) });
await write('decoders', "mesa.mjs's decoders of other ships' nouns (its decoders export, with cord as the file reader), and furum.mjs's and forum.mjs's reads and notes, given the host's grubbery: files {\"<folder> <name>\": noun} or a deep peek's {\"<folder>\": [[path, noun]]}", [
  C('json', N(jsonN)), C('json', N(['s', ''])), C('json', N(['n', '1.5e3'])), C('json', N(null)),
  C('tang', N(tang)), C('tangText', N(tang.slice(1))), C('tang', N(5)),
  C('pathOf', N(path('/a/b/c'))), C('pathOf', N(null)), C('segments', N(path('/x/~zod/0v1.abc'))),
  C('measure', N([1, [2, 3]])), C('list', N([1, 2, 3, null])), C('entries', N(new Map([['a', 1], ['b', 2]]))),
  C('fileOf', N(fs), N(fsilo)), C('fileOf', N([fs[0], [['temp', null, fs[1][2]], fs[2]]]), N(fsilo)),
  C('fileOf', N([fs[0], [['tomb', null], null]]), N(fsilo)), C('fileOf', N(fs), N([new Map(), new Map()])),
  C('ballOf', N(ds), N(dsilo)), C('ballOf', N(fs), N(fsilo)),
  C('ship', N(0n)), C('ship', N(256n)), C('ship', N(0x10000n)), C('ship', N(2n ** 64n + 5n)),
  C('furum.nextPost', furumFiles, 'news'), C('furum.nextPost', furumFiles, 'none'),
  C('furum.posts', furumFiles, 'news'), C('furum.posts', furumFiles, 'none'),
  C('furum.comments', furumFiles, 'news', 0), C('furum.comments', furumFiles, 'news', 5),
  C('furum.votes', furumFiles, 'news', 0), C('furum.votes', furumFiles, 'news', 1),
  C('forum.board', dirsN(forumDirs()), 'm1btest'), C('forum.board', dirsN(forumDirs()), 'other'),
  C('forum.thread', dirsN(forumDirs()), 'm1btest', 1), C('forum.thread', dirsN(forumDirs()), 'm1btest', 9),
  C('forum.board', dirsN(forumDirs({ content: [[['posts', 'b1'], [2, new Map()]]] })), 'm1btest'),
  C('forum.board', dirsN(forumDirs({ content: [[['votes', 'b1'], [1, new Map([[['reply', 1], [null, null]]])]]] })), 'm1btest'),
  C('forum.note', note('~zod', [DESK_BASE, 'forum.sig'], ['/lattice', 'forum-note'], ['m1btest', 'forum: you are acting too fast, slow down and retry'])),
  C('forum.note', note('~nec', [DESK_BASE, 'forum.sig'], ['/lattice', 'forum-note'], ['m1btest', 'from another ship'])),
  C('forum.note', note('~zod', [DESK_BASE, 'forum.sig'], ['/lattice', 'forum-note'], [['m1btest', 1], 'a cell for a board'])),
  C('furum.note', note('~zod', [FURUM_BASE, 'inbox.sig'], ['/furum', 'msg'], ['note', '~zod refused your change', 'post not found', null, null])),
  C('furum.note', note('~zod', [FURUM_BASE, 'inbox.sig'], ['/furum', 'msg'], ['act', 'x'])),
]);

// ------------------------------------------------------------ hostile, by hand

const hn = hostileNouns();
for (const c of hn) {
  const r = await evaluate(urb, c);
  const ok = c.error !== undefined ? r.error === c.error : JSON.stringify(r.out) === JSON.stringify(c.out);
  if (!ok || r.ms > quick) {
    throw new Error(`hostile ${c.fn}: wanted ${JSON.stringify(c.error ?? c.out)} within ${quick} ms, got ${JSON.stringify(r).slice(0, 300)}`);
  }
}
writeFileSync(`${here}hostile-nouns.json`, JSON.stringify({
  about: `hand-written hostile nouns (hostile.mjs): each must be refused, with exactly its error, within ${quick} ms, and never crash or hang`,
  quick, cases: hn,
}, null, 1) + '\n');
console.log(`hostile-nouns.json: ${hn.length} cases`);

// ------------------------------------------------------------ the extension's renderers

if (opt.ext) {
  const lib = await extLib(opt.ext);
  const commit = execFileSync('git', ['-C', opt.ext, 'rev-parse', '--short=7', 'HEAD'], { encoding: 'utf8' }).trim();
  const r = JSON.parse(readFileSync(`${here}inputs/render.json`, 'utf8'));
  await write('render-ext', `nisfeb-extension's lib/gemtext.js and lib/markdown.js: gemtext and markdown's blocks, mdNodes' nodes (lib.mjs's document, links by linkHref on ${PAGE}), spans, safeLink, linkTarget and linkHref on ${PAGE}, readerPath`, [
    ...r.bodies.flatMap(b => [C('gemtext', b), C('markdown', b), C('mdNodes', b)]),
    ...r.lines.map(l => C('spans', l)),
    ...r.links.flatMap(t => [C('safeLink', t), C('linkTarget', PAGE, t), C('linkHref', PAGE, t)]),
    C('readerPath', 'urb://~zod'), C('readerPath', 'urb://~zod/a b?c#d'),
  ], { extension: commit }, lib);
  const rr = randomRender();
  writeFileSync(`${here}inputs/random.json`, JSON.stringify(rr, null, 1) + '\n');
  await write('render-ext-random', `nisfeb-extension's renderers on random bodies and links (random.mjs, inputs/random.json): gemtext and markdown's blocks and mdNodes' nodes as in render-ext, safeLink, linkTarget and linkHref on ${PAGE}`, [
    ...rr.bodies.flatMap(b => [C('gemtext', b), C('markdown', b), C('mdNodes', b)]),
    ...rr.links.flatMap(t => [C('safeLink', t), C('linkTarget', PAGE, t), C('linkHref', PAGE, t)]),
  ], { extension: commit }, lib);
  const hr = hostileRender();
  for (const c of hr) {
    const got = await evaluate(urb, c, lib);
    if (JSON.stringify(got.out) !== JSON.stringify(c.out) || got.ms > quick) {
      throw new Error(`hostile render ${c.fn} ${JSON.stringify(c.in).slice(0, 80)}: wanted ${JSON.stringify(c.out)}, got ${JSON.stringify(got).slice(0, 300)}`);
    }
  }
  writeFileSync(`${here}hostile-render.json`, JSON.stringify({
    about: `hand-written hostile page bodies and links (hostile.mjs): what the extension (lib.mjs) and the gateway (conformance_test.go) must each give, within ${quick} ms. safeLink(target); mdLink(text, target), whether [text](target) is a link; mdFirst(body), the first block's tag (pre when the body is over the 256 KB cap); escapes(mark, body), that no raw tag gets through; renders(mark, body), that it renders at all`,
    extension: commit, quick, cases: hr,
  }, null, 1) + '\n');
  console.log(`hostile-render.json: ${hr.length} cases`);
}

await urb.close();
process.exit(0);
