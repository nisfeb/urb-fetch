// The hostile vectors, written by hand against the hardened JS
// (docs/rust-core.md, rule 4): each input, and the error it must give.
// They test the caps, so nothing here is taken from what the code does:
// gen.mjs only jams the nouns and checks the JS passes every one.
//
// Nouns are built as plain trees (jam.mjs: an atom a BigInt, a cell
// [head, tail]), so shared structure stays shared in the jam: a noun
// vast as a tree is a few bytes on the wire.

import { hex, jam } from '../wasm/jam.mjs';
import { DESK_BASE } from '../wasm/mesa.mjs';
import { FURUM_BASE } from '../wasm/furum.mjs';

export const quick = 2000;   // ms: a refusal takes no longer
// a decoder's error, word for word
const H = what => `another ship's noun has ${what}`;
const OVER_BYTES = 'over 64 MiB of atoms';
// what a decoder prints past the atoms it read: a tank's separators, a
// directory's paths
const OVER_PRINT = "another ship's noun reads as over 64 MiB";

const T = t => ({ $noun: hex(jam(t)) });
const a = x => (typeof x === 'string' ? [...new TextEncoder().encode(x)].reduceRight((v, b) => (v << 8n) | BigInt(b), 0n) : BigInt(x));
const list = (...xs) => xs.reduceRight((t, x) => [x, t], 0n);
const tuple = (...xs) => xs.slice(0, -1).reduceRight((t, x) => [x, t], xs.at(-1));
// a map (a treap [n l r]) of 2^n - 1 entries as a tree, in 2n cells
const sharedMap = (entry, n) => { let m = 0n; for (let i = 0; i < n; i++) m = [entry, [m, m]]; return m; };
// a cell that is 2^n copies of x as a tree, in n cells
const share = (x, n) => { let t = x; for (let i = 0; i < n; i++) t = [t, t]; return t; };

export function hostileNouns() {
  const num = n => [a('n'), a(String(n))];
  const arr = (...xs) => [a('a'), list(...xs)];
  let deep = num(1);
  for (let i = 0; i < 64; i++) deep = arr(deep);   // 65 deep with the number
  let wide = num(1);
  for (let i = 0; i < 40; i++) wide = arr(wide, wide);
  const map = sharedMap([a('k'), num(1)], 30);
  const tape = s => list(...[...s].map(c => a(c.charCodeAt(0))));
  const leaf = [a('leaf'), tape('hi')];
  let rose = leaf;
  for (let i = 0; i < 40; i++) rose = [a('rose'), [[tape(' '), [0n, 0n]], list(rose, rose)]];
  //  a %snap of a file, pace [%firm ~ 7], and silos to go with it
  const snap = [0n, [[a('firm'), [0n, 7n]], 0n]];
  const silo = (nouns, jects) => [nouns, jects];
  const one = (k, v) => [[k, v], [0n, 0n]];   // a map of one entry
  //  a directory's snap and the tree jects it names
  const tree = (fil, dir) => [a('tree'), [0n, [0n, [0n, [fil, dir]]]]];
  const dirSnap = top => [0n, [[a('firm'), [0n, top]], 0n]];
  const self = silo(0n, one(5n, [0n, tree(0n, one(a('loop'), [5n, 0n]))]));
  let chain = 0n;
  const jects = [];
  for (let i = 66; i >= 1; i--) jects.push([BigInt(i), [0n, tree(0n, i < 66 ? one(a('d'), [BigInt(i + 1), 0n]) : 0n)]]);
  for (const [k, v] of jects) chain = [[k, v], [chain, 0n]];
  //  trees 1..20, each holding the next twice: 2^20 trees in 20 jects
  let fan = 0n;
  for (let i = 20; i >= 1; i--) {
    const kids = i < 20 ? [[a('x'), [BigInt(i + 1), 0n]], [0n, [[a('y'), [BigInt(i + 1), 0n]], [0n, 0n]]]] : 0n;
    fan = [[BigInt(i), [0n, tree(0n, kids)]], [fan, 0n]];
  }
  //  a map (a treap leaning one way) of [key value] pairs
  const mapOf = kvs => kvs.reduce((m, kv) => [kv, [m, 0n]], 0n);
  const big8k = a('x'.repeat(8192)), str8k = [a('s'), big8k];
  const longSep = [a('rose'), [[list(...Array(8200).fill(44n)), [0n, 0n]], list(...Array(8200).fill(leaf))]];
  //  n directories, or n files, n0 to n(n-1), naming one jobe
  const names = n => Array.from({ length: n }, (_, i) => a(`n${i}`));
  const dirs = (n, jobe) => mapOf(names(n).map(k => [k, [jobe, 0n]]));
  const files = (n, jobe) => mapOf(names(n).map(k => [k, jobe]));
  const LB = `${DESK_BASE}/forum/boards/b`;
  const card = [1n, tuple(a('b'), a('t'), a('d'), 0n, 0n, a('poster'), 0n)];
  const goodPub = [[list(a('card')), card], [list(a('conf')), [1n, [1n, 0n]]]];
  const board = (pub, content = []) => ({
    [`${LB}/pub`]: pub.map(([p, v]) => [ptext(p), T(v)]),
    [`${LB}/content`]: content.map(([p, v]) => [ptext(p), T(v)]),
  });
  const FB = `${FURUM_BASE}/boards/b`;
  return [
    { fn: 'measure', in: [T(share(1n, 18))], error: H('over 200000 cells') },
    { fn: 'measure', in: [T(share(1n, 200))], error: H('over 200000 cells') },
    { fn: 'json', in: [T(arr(deep))], error: H('$json nested over 64 deep') },
    { fn: 'json', in: [T(wide)], error: H('over 200000 cells') },
    { fn: 'json', in: [T([a('x'), 1n])], error: H('$json of no kind we know') },
    { fn: 'json', in: [T([a('b'), 2n ** 512n])], error: H('a number over 32 bytes') },
    { fn: 'json', in: [T([[1n, 2n], 1n])], error: H('a cell where an atom goes') },
    { fn: 'json', in: [T([a('o'), map])], error: H('over 200000 cells') },
    { fn: 'entries', in: [T(map)], error: H('a map over 200000 entries') },
    { fn: 'pathOf', in: [T(list(...Array(65).fill(a('a'))))], error: H('a path over 64 segments') },
    { fn: 'pathOf', in: [T(list(a('x'.repeat(257))))], error: H('a path segment over 256 bytes') },
    { fn: 'pathOf', in: [T(list([1n, 2n]))], error: H('a cell where an atom goes') },
    //  a tang prints what was wrong with it, as text: it never throws
    { fn: 'tang', in: [T([a('err'), list(rose)])], out: H('over 200000 cells') },
    { fn: 'fileOf', in: [T(snap), T(silo(0n, [[2n ** 300n, 0n], [0n, 0n]]))], error: H('a number over 32 bytes') },
    { fn: 'fileOf', in: [T([0n, [[[1n, 2n], 0n], 0n]]), T(0n)], error: H('a cell where an atom goes') },
    { fn: 'fileOf', in: [T(snap), T(silo(0n, sharedMap([8n, [0n, 0n]], 20)))], error: H('a map over 200000 entries') },
    { fn: 'ballOf', in: [T(dirSnap(5n)), T(self)], error: H('a directory that holds itself') },
    { fn: 'ballOf', in: [T(dirSnap(1n)), T(silo(0n, chain))], error: H('a directory over 64 deep') },
    { fn: 'ballOf', in: [T(dirSnap(1n)), T(silo(0n, fan))], error: H('a directory of over 200000 trees and files') },
    { fn: 'ship', in: [T(2n ** 200n)], error: H('a ship over 128 bits') },
    //  one atom at many places counts at each: [%a 8200 × [%s <8 KiB>]]
    //  is 67 MB as a tree
    { fn: 'measure', in: [T(arr(...Array(8200).fill(str8k)))], error: H(OVER_BYTES) },
    { fn: 'json', in: [T(arr(...Array(8200).fill(str8k)))], error: H(OVER_BYTES) },
    { fn: 'json', in: [T([a('o'), mapOf(Array(8200).fill([big8k, num(1)]))])], error: H(OVER_BYTES) },
    //  a separator of 8200 characters between each two of 8200 kids
    { fn: 'tang', in: [T([a('err'), list(longSep)])], out: OVER_PRINT },
    { fn: 'tangText', in: [T(list(longSep))], error: OVER_PRINT },
    //  measured and printed from one budget: 40 MB of atoms in a leaf,
    //  then 33 MB of separators, though each alone is under it
    { fn: 'tangText', in: [T(list([a('leaf'), list(...Array(5000).fill(big8k))],
      [a('rose'), [[list(...Array(8200).fill(44n)), [0n, 0n]], list(...Array(4000).fill(leaf))]]))], error: OVER_PRINT },
    //  500 directories naming one tree, whose own 500 are all missing
    { fn: 'ballOf', in: [T(dirSnap(1n)), T(silo(0n, mapOf([[1n, [0n, tree(0n, dirs(500, 2n))]], [2n, [0n, tree(0n, dirs(500, 99n))]]])))],
      error: H('a directory of over 200000 trees and files') },
    //  4200 files naming one 16 KiB noun
    { fn: 'ballOf', in: [T(dirSnap(1n)), T(silo(mapOf([[4n, [0n, a('x'.repeat(16384))]]]),
      mapOf([[1n, [0n, tree(files(4200, 3n), 0n)]], [3n, [0n, [a('leaf'), [4n, 0n]]]]])))], error: H(OVER_BYTES) },
    //  5000 files 63 directories down, each named by 256 bytes
    { fn: 'ballOf', in: [T(dirSnap(1n)), T(silo(mapOf([[4n, [0n, 1n]]]), mapOf([...Array.from({ length: 63 }, (_, i) =>
      [BigInt(i + 1), [0n, tree(0n, mapOf([[a('d'.repeat(256)), [BigInt(i + 2), 0n]]]))]]),
      [64n, [0n, tree(files(5000, 100n), 0n)]], [100n, [0n, [a('leaf'), [4n, 0n]]]]])))], error: OVER_PRINT },
    //  each segment of a path counts, even an empty one: 4090 files of
    //  16 KiB, 63 empty directories down (at the top, they would read)
    { fn: 'ballOf', in: [T(dirSnap(1n)), T(silo(mapOf([[4n, [0n, a('x'.repeat(16384))]]]), mapOf([...Array.from({ length: 63 }, (_, i) =>
      [BigInt(i + 1), [0n, tree(0n, mapOf([[0n, [BigInt(i + 2), 0n]]]))]]),
      [64n, [0n, tree(mapOf(Array.from({ length: 4090 }, (_, i) => [a(`f${String(i).padStart(4, '0')}`), 100n])), 0n)]],
      [100n, [0n, [a('leaf'), [4n, 0n]]]]])))], error: H(OVER_BYTES) },
    { fn: 'forum.board', in: [board([[list(a('card')), [2n, 0n]]]), 'b'], error: "~zod's board grub is unreadable: /pub/card (not version 1)" },
    { fn: 'forum.board', in: [board(goodPub, [[list(a('posts'), a('b0')), [1n, one(2n ** 300n, 0n)]]]), 'b'], error: "~zod's board grub is unreadable: /content/posts/b0 (another ship's noun has a number over 32 bytes)" },
    { fn: 'forum.board', in: [board(goodPub, [[list(a('votes'), a('b0')), [1n, one([a('reply'), 1n], [0n, 0n])]]]), 'b'], error: "~zod's board grub is unreadable: /content/votes/b0 (no vote target %reply)" },
    { fn: 'forum.board', in: [board([[list(a('roles')), [1n, one(1n, a('king'))]]]), 'b'], error: "~zod's board grub is unreadable: /pub/roles (no role king)" },
    { fn: 'forum.board', in: [board([[list(a('conf')), [1n, [1n, 0n]]]]), 'b'], error: "~zod's board has no card" },
    { fn: 'furum.posts', in: [{ [`${FB}/pub conf`]: T([1n, [2n ** 40n, 0n]]) }, 'b'], error: "~zod's board b claims 1099511627776 posts, over 100000" },
  ];
}
const ptext = p => { const out = []; for (let t = p; Array.isArray(t); t = t[1]) out.push(new TextDecoder().decode(bytes(t[0]))); return out; };
const bytes = v => { const b = []; for (let x = v; x > 0n; x >>= 8n) b.push(Number(x & 0xffn)); return Uint8Array.from(b); };

// The renderers' hostile inputs, for the extension and the gateway alike
//   {"$repeat": [s, n]} is s n times, and {"$cat": [...]} the parts
//   joined, so a long body stays short in the file
const R = (str, n) => ({ $repeat: [str, n] });
const cat = (...parts) => ({ $cat: parts });
export function hostileRender() {
  const out = [];
  const refused = ['javascript:alert(1)', 'JAVASCRIPT:alert(1)', 'JaVaScRiPt:x', ' javascript:x', 'data:text/html,<b>x', 'DATA:x',
    'vbscript:x', 'VBScript:x', 'file:///etc/passwd', 'chrome://settings', '//evil.example/x', '\\\\evil.example/x', '/\\evil.example',
    '\\/evil.example', ' //evil.example', 'urb:x\x00',
    //  schemes match in ASCII only (Go's (?i) folds ſ into s), and a
    //  link is protocol-relative after any of JavaScript's whitespace
    'httpſ://evil.example/x', 'HTTPſ://evil.example', '\u00a0//evil.example', '\u3000\\\\evil.example', '\ufeff//evil.example'];
  for (let c = 0; c <= 0x20; c++) {
    const ch = String.fromCharCode(c);
    refused.push(`${ch}https://a.example/x`, `https://a.example/${ch}x`, `https://a.example/x${ch}`, `${ch}//evil.example`);
  }
  refused.push('\x7fhttps://a.example', 'https://a.example/\x7f', 'https://a.example\x7f');
  for (const t of refused) out.push({ fn: 'safeLink', in: [t], out: false });
  for (const t of ['http://a.example', 'HTTPS://A.EXAMPLE/x', 'urb://~zod', 'URB://~zod/x', 'gemini://g.example', 'mailto:a@b.example', '/x', 'x/y', '?q', '#f', '..'])
    out.push({ fn: 'safeLink', in: [t], out: true });
  //  link text at most 256, target at most 512, counted in UTF-16 units
  const t256 = 'a'.repeat(256), u512 = '/' + 'b'.repeat(511), smile = '\u{1F600}'.repeat(128);
  out.push({ fn: 'mdLink', in: [t256, '/x'], out: true }, { fn: 'mdLink', in: [t256 + 'a', '/x'], out: false },
    { fn: 'mdLink', in: ['t', u512], out: true }, { fn: 'mdLink', in: ['t', u512 + 'b'], out: false },
    { fn: 'mdLink', in: [smile, '/x'], out: true }, { fn: 'mdLink', in: [smile + 'a', '/x'], out: false },
    { fn: 'mdLink', in: ['t', 'javascript:x'], out: false });
  //  a Markdown body over 256 KB (in UTF-16 units) is text in a <pre>
  out.push({ fn: 'mdFirst', in: [R('a', 256 * 1024)], out: 'p' }, { fn: 'mdFirst', in: [R('a', 256 * 1024 + 1)], out: 'pre' },
    { fn: 'mdFirst', in: [cat(R('\u{1F600}', 128 * 1024), 'a')], out: 'pre' });
  //  raw HTML is text
  for (const mark of ['md', 'gmi', 'text']) {
    for (const b of ['<script>alert(1)</script>', '# <img src=x onerror=alert(1)>', '[<b>x</b>](/y)', '=> /x <script>', '* <iframe src=//e>'])
      out.push({ fn: 'escapes', in: [mark, b], out: true });
  }
  //  lines that make a backtracking regex slow, and must not here
  const n = 100_000;
  for (const b of [R('[', n), R('](', n), R('[a](', n), R('*', n), R('**', n), R('`', n), R('_', n),
    cat('#', R(' ', n), 'x'), cat(R(' ', n), '#'), cat('# ', R('#', n), ' x'), R('- ', n), R('> ', n), cat('=> ', R(' ', n), 'x'),
    cat('[', R('a', n), ']('), cat('[a](', R('b', n)), cat(R('\t', n), '<')])
    for (const mark of ['md', 'gmi']) out.push({ fn: 'renders', in: [mark, b], out: true });
  return out;
}
