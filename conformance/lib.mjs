// What the JS side of the conformance vectors shares: an engine to build
// and read nouns with, how each vector's function runs (evaluate), and
// the extension's renderers.
//
// A noun in a vector is {"$noun": jam hex}, a long text {"$repeat": [s,
// n]} or {"$cat": [parts]}; every other value is in trace.mjs's encoding
// ($big, $hex, $date, $map, $set, $u).

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { forum } from '../wasm/forum.mjs';
import { furum } from '../wasm/furum.mjs';
import { asNoun, cue, hex, jam, unhex } from '../wasm/jam.mjs';
import { da, decoders, deUrb, fakeKeyfile, looksHtml, MAX_BYTES, MAX_CELLS, messages, open, renderAs, snapOf, uv, waveOf } from '../wasm/mesa.mjs';
import { actions } from '../wasm/forum.mjs';
import { dec, enc } from '../wasm/trace.mjs';

export const PAGE = 'urb://~zod/a/b';   // the page the link vectors are on
export const NOW = 1761600000000;       // the clock furum and forum notes read

const wasm = readFileSync(new URL('../wasm/build/mesa.wasm', import.meta.url));

// an engine that sends nothing, for its nouns
export async function engine() {
  const keyfile = await fakeKeyfile(wasm, '~zod');
  return open({
    wasm, fake: true, log: () => {},
    transport: { start() {}, send() {}, close() {} },
    store: { load: () => ({ keyfile, rows: null, fresh: true }), saveKey() {}, commit() {} },
  });
}

// an engine for furum.mjs and forum.mjs whose grubbery is the vector's:
// peek(ship, folder, name, conv) and poke(...) as urb.grub has them
export function fakeUrb(urb, { peek = async () => null, poke = async () => ({ ok: true }) } = {}) {
  let inbox = null;
  return {
    our: '~nec', now: () => NOW, entropy: n => new Uint8Array(n), nouns: urb.nouns,
    grub: { peek, poke, onInbox: f => { inbox = f; } },
    hear: msg => inbox?.(msg),
  };
}

// a vector's arguments as JS: {$noun} as a noun the decoders read
export function args(x) {
  if (x && typeof x === 'object' && !Array.isArray(x) && '$noun' in x) return asNoun(cue(unhex(x.$noun)));
  if (x && typeof x === 'object' && '$repeat' in x) return x.$repeat[0].repeat(x.$repeat[1]);
  if (x && typeof x === 'object' && '$cat' in x) return x.$cat.map(args).join('');
  if (Array.isArray(x)) return x.map(args);
  if (x && typeof x === 'object' && !Object.keys(x).some(k => k.startsWith('$'))) {
    return Object.fromEntries(Object.entries(x).map(([k, v]) => [k, args(v)]));
  }
  return dec(x);
}

// a grubbery for furum and forum reads: files, {"<folder> <name>": noun};
// dirs, {"<folder>": [[path, noun]]} for a deep peek; null for neither
// (and lattice's /pub/index, which forum.mjs finds the host's base by)
const grubbery = (files = {}, dirs = {}) => async (ship, folder, name, conv) => {
  if (name === null) return dirs[folder] ? conv(dirs[folder]) : null;
  if (name === 'index' && folder.endsWith('/pub') && !files[`${folder} ${name}`]) return conv(null);
  const n = files[`${folder} ${name}`];
  return n ? conv(n) : null;
};

// how each fn runs, on its arguments as args() gives them
function fns(urb, lib) {
  const { sham, tree, mug } = urb.nouns;
  const cord = n => (n.isCell ? null : n.text());
  const f = {
    deUrb, renderAs, looksHtml, uv, da,
    sham: x => sham(x).toString(16), mug, noun: x => tree(x),
    peek: messages.peek, poke: messages.poke, pack: messages.pack, want: messages.want, snap: messages.snap,
    snapFile: (w, d, id, file) => messages.snap(w, d, id, ...snapOf(file, sham)),
    intakeOf: (w, d, file) => messages.intake(w, d, waveOf(file, 'gmi')),
    json: decoders.json, tang: decoders.tang, pathOf: decoders.pathOf,
    //  as on_poke reads a %pack: measured, then printed, from one budget
    tangText: n => {
      const b = { cells: MAX_CELLS, bytes: MAX_BYTES };
      decoders.measure(n, b);
      return decoders.tangText(n, b);
    },
    segments: decoders.segments, measure: n => (decoders.measure(n), true),
    list: n => [...decoders.list(n)].length, entries: n => decoders.entries(n).length,
    fileOf: (snap, silo) => decoders.fileOf(snap, silo, cord),
    ballOf: (snap, silo) => decoders.ballOf(snap, silo, files => files.map(([p, n]) => [p, cord(n)])),
    ship: n => urb.nouns.ship(n),
    'furum.nextPost': (files, name) => furum(fakeUrb(urb, { peek: grubbery(files) }), '~zod').nextPost(name),
    'furum.posts': (files, name) => furum(fakeUrb(urb, { peek: grubbery(files) }), '~zod').posts(name),
    'furum.comments': (files, name, post) => furum(fakeUrb(urb, { peek: grubbery(files) }), '~zod').comments(name, post),
    'furum.votes': (files, name, post) => furum(fakeUrb(urb, { peek: grubbery(files) }), '~zod').votes(name, post),
    'forum.board': (dirs, name) => forum(fakeUrb(urb, { peek: grubbery({}, dirs) }), '~zod').board(name),
    'forum.thread': (dirs, name, post) => forum(fakeUrb(urb, { peek: grubbery({}, dirs) }), '~zod').thread(name, post),
    'furum.note': msg => {
      const u = fakeUrb(urb);
      let got = null;
      furum(u, '~zod').onNote(n => { got = n; });
      u.hear(msg);
      return got;
    },
    'forum.note': msg => {
      const u = fakeUrb(urb);
      let got = null;
      forum(u, '~zod').onNote(n => { got = n; });
      u.hear(msg);
      return got;
    },
  };
  for (const [k, a] of Object.entries(actions)) f[`forum.${k}`] = a;
  if (lib) {
    Object.assign(f, {
      gemtext: lib.gemtext, markdown: lib.markdown, spans: lib.spans, safeLink: lib.safeLink,
      linkTarget: lib.linkTarget, linkHref: lib.linkHref, readerPath: lib.readerPath,
      mdNodes: body => nodesOf(lib, body),
      //  the hostile renders' questions (hostile.mjs)
      mdLink: (text, target) => lib.spans(`[${text}](${target})`).some(s => 'link' in s),
      mdFirst: body => nodesOf(lib, body)[0].tag,
      escapes: (mark, body) => {
        if (mark !== 'md') return true;   // gemtext's and text's blocks are text, set as text
        const tags = n => (n.tag ? [n.tag, ...n.kids.flatMap(tags)] : []);
        return nodesOf(lib, body).flatMap(tags).every(t => SAFE_TAGS.has(t));
      },
      renders: (mark, body) => (mark === 'md' ? nodesOf(lib, body) : lib.gemtext(body), true),
    });
  }
  return f;
}

// the elements mdNodes may make
const SAFE_TAGS = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'li', 'blockquote', 'pre', 'code', 'strong', 'em', 'a', 'hr']);

// a message's noun, as jam hex: the messages' output
const NOUN_OUT = new Set(['noun', 'peek', 'poke', 'pack', 'want', 'snap', 'snapFile', 'intakeOf',
  ...Object.keys(actions).map(k => `forum.${k}`)]);

// {out} or {error, ms}: one case, run. A noun out is jam hex.
export async function evaluate(urb, c, lib) {
  const f = fns(urb, lib)[c.fn];
  if (!f) return { skip: `no ${c.fn} here` };
  const t = Date.now();
  try {
    let v = await f(...args(c.in));
    if (NOUN_OUT.has(c.fn)) v = hex(jam(urb.nouns.tree(v)));
    return { out: enc(v), ms: Date.now() - t };
  } catch (e) {
    return { error: e.message, ms: Date.now() - t };
  }
}

// the extension's lib/gemtext.js and lib/markdown.js
export async function extLib(dir) {
  const lib = name => import(pathToFileURL(resolve(dir, 'lib', name)).href);
  return { ...(await lib('markdown.js')), ...(await lib('gemtext.js')) };
}

// mdNodes' nodes, with a document whose elements are plain objects:
// {tag, attrs, kids}, and text nodes {text}
export function nodesOf(lib, body) {
  //  accessors defined as accessors: Object.assign would call each
  //  getter once and copy its value, so text set by textContent and an
  //  a's href and rel were lost. Setting textContent to '' leaves no
  //  child, as in a DOM.
  const el = tag => {
    const e = { tag, kids: [], attrs: {} };
    return Object.defineProperties(e, Object.getOwnPropertyDescriptors({
      appendChild(k) { e.kids.push(k); return k; },
      append(...ks) { for (const k of ks) e.kids.push(typeof k === 'string' ? { text: k } : k); },
      setAttribute(k, v) { e.attrs[k] = String(v); },
      set textContent(t) { e.kids = String(t) === '' ? [] : [{ text: String(t) }]; },
      get textContent() { return e.kids.map(k => k.text ?? '').join(''); },
      set href(v) { e.attrs.href = String(v); },
      set className(v) { e.attrs.class = String(v); },
      set rel(v) { e.attrs.rel = String(v); },
    }));
  };
  const doc = { createElement: el, createTextNode: t => ({ text: String(t) }), createDocumentFragment: () => el('#fragment') };
  const strip = n => (n.text !== undefined && !n.tag ? { text: n.text }
    : { tag: n.tag, ...(Object.keys(n.attrs).length ? { attrs: n.attrs } : {}), kids: n.kids.map(strip) });
  const got = lib.mdNodes(doc, body, t => lib.linkHref(PAGE, t));
  return Array.isArray(got) ? got.map(strip) : strip(got);
}

void [renderAs, deUrb];
