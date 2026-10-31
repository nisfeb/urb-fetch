// mesa.mjs: libmesa's engine (build/mesa.wasm, see build.sh) as a comet,
// with lattice's page reader on it, for any JavaScript host: Node, an
// extension's service worker, a page. It reads a page the way
// ../prototype/urb_fetch.py does:
//
//   1. resolve the url as lattice's +de-urb does (the published forms only)
//   2. read the publisher's /pub/revs over grubbery's cross-ship peek
//      (a %grubbery-load poke, answered by a %grubbery-transfer poke back)
//   3. %keen the page body at /g/x/1/grubbery//1/pub/page/<rel>/<id>
//   4. if the list has no entry or the keen misses, peek the vault file
//
// The host brings:
//
//   wasm       mesa.wasm's bytes, or a compiled WebAssembly.Module
//   transport  where datagrams go: {start(ship, onPacket), send(lane, bytes),
//              close()}. A lane is {galaxy, name} or {ip, port}; onPacket
//              takes (ip, port, bytes). relayTransport below is one, over a
//              WebSocket to `urb-fetch -relay`; host.mjs has one for UDP.
//   store      where the identity and the engine's state are kept:
//              {load() -> {keyfile, rows, fresh}, saveKey(keyfile),
//              commit(rows)}, each may return a promise. rows are glue.c's
//              [klen:4][key][vlen:4][val]; readRows and writeRows convert.
//              With no keyfile, a comet is mined and saved. A moon's
//              keyfile comes from the host, with fresh true until its first
//              commit is saved: a moon may start from empty state only if it
//              never ran anywhere.
//
//   now        optional: the clock, () -> milliseconds since the epoch
//              (Date.now by default)
//   entropy    optional: n -> a Uint8Array of n random bytes (from
//              crypto.getRandomValues by default). It gives the wire nonce,
//              the ids of the %snaps we answer and a mined comet's seed;
//              pub.mjs takes its page ids from it. With both fixed, a
//              session's inputs decide its outputs.
//   timers     optional: {setTimeout(f, ms, what) -> id, clearTimeout(id)},
//              every timer the engine sets, the globals by default. what
//              names it: 'wake', 'point <ship>' (a fake ship's keys),
//              'roller <ship>' (a key lookup's 30 s), 'peek <wire>',
//              'poke <wire>', 'keen <path>' or 'close'
//   fetch      optional: fetch(url, opts) for the roller's key lookups and
//              the comet stars, the global by default. trace.mjs gives all
//              four to record a session and replay it.
//
//   const urb = await open({ wasm, transport, store })
//   const { key, body, how, mark } = await urb.read('urb://~sampel-palnet/page')
//
// mark is the page's format, the mark it was published under: render it
// as renderAs(mark, body) says. /k/ and /t/ reads have no mark.
//
// It also serves, for pub.mjs: our files to other ships' grubbery (their
// peeks and keeps, see grub.serve) and our scry namespace (scry.serve).

export const DESK_BASE = '/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app';
const APP_BASE = '/apps/lattice.lattice_app';
const REVS_TTL = 60_000;     // +revs-ttl
const KEEN_TIMEOUT = 10_000; // +mesa-timeout
const PEEK_TIMEOUT = 30_000; // +remote-timeout
const ROLLER = 'https://roller.urbit.org/v1/azimuth';
const STARS = 'https://bootstrap.urbit.org/comet-stars.jam';
const KNOT = /^[a-z0-9~._-]+$/;
const UD = /^(0|[1-9][0-9]{0,2}(\.[0-9]{3})*)$/;   // an @ud as +slaw reads it: 1.000, never 1000
const NONE = 2n ** 64n - 1n;
const ACK = 0;
const LANE_GALAXY = 1, LANE_IP = 2;
const GONE = 0xffffffff;     // a deleted row's vlen
// ponytail: a cap on the keeps we answer, so a ship can't grow our state
// without bound; past it a keep gets its first wave and no news
const MAX_KEEPERS = 256;

// wasm hands 64-bit integers to JS as signed BigInts: read them unsigned
const u64 = x => BigInt.asUintN(64, x);
const enc = new TextEncoder(), dec = new TextDecoder();
const bigOf = bytes => bytes.reduceRight((v, b) => (v << 8n) | BigInt(b), 0n);   // little-endian

export class UrbError extends Error {}

// a BigInt as @uv text, as (scot %uv n) writes it: 0v, then base-32 digits
// in dotted groups of five from the right
export function uv(n) {
  const d = n.toString(32), head = d.length % 5 || 5;
  return '0v' + [d.slice(0, head), ...(d.slice(head).match(/.{5}/g) || [])].join('.');
}

// milliseconds since the epoch as a @da
export const da = ms => ((BigInt(Math.floor(ms / 1000)) + 0x8000000cce9e0d80n) << 64n)
  | ((BigInt(ms % 1000) << 64n) / 1000n);

// ------------------------------------------------------------- addresses

// What an urb:// url names, as lattice's +de-urb resolves it: [ship, rel,
// mount]. Mount 'n' is a published page (rel under the pub vault); 'k' a
// know entry (rel is its key); 't' a file anywhere in the publisher's
// tree (rel is its absolute path); 'p' one of lattice's live pages. All
// but 'n' are private: the publisher grants who may read them. Mount
// 'forum' is a forum board, rel [board] or [board, post] with post a
// number, or a BigInt past 2^53 - 1 (forum.mjs reads it): since release 73 forum/<board>[/<post>]
// names a board, and any other shape under forum/ is still a page.
export function deUrb(url) {
  if (!url.startsWith('urb://')) throw new UrbError('not an urb:// url');
  const rest = url.slice(6);
  const slash = rest.indexOf('/');
  const who = slash < 0 ? rest : rest.slice(0, slash);
  let tail = slash < 0 ? '' : rest.slice(slash + 1);
  if (tail.endsWith('/')) tail = tail.slice(0, -1);   // urb://~s/site/ = urb://~s/site
  if (!/^~[a-z]+(-[a-z]+)*$/.test(who)) throw new UrbError(`no ship in ${url}`);
  if (!tail) return [who, ['index'], 'n'];
  const segs = tail.split('/');
  if (!segs.every(s => KNOT.test(s))) throw new UrbError(`bad path in ${url}`);
  if (segs[0] === 'forum' && segs.length > 1) {
    if (segs.length === 2) return [who, [segs[1]], 'forum'];
    if (segs.length === 3 && UD.test(segs[2])) {
      const id = BigInt(segs[2].replaceAll('.', ''));   // exact: a BigInt past 2^53 - 1
      return [who, [segs[1], id <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(id) : id], 'forum'];
    }
  }
  if (segs[0].length > 1) return [who, segs, 'n'];     // frozen legacy form
  if ('nkpt'.includes(segs[0])) return [who, segs.slice(1), segs[0]];
  throw new UrbError(`/${segs[0]}/ is not a mount`);
}

// lattice-gmi's +looks-html: the first non-blank character is <
export const looksHtml = body => body.slice(0, 256).trimStart().startsWith('<');

// lattice's view modes (view-mode:pg). Release 74 and later publish each
// page under its mode's mark (+mode-mark); older ones publish every page
// as %gmi (nisfeb/lattice#321).
export const PAGE_MARKS = new Set(['text', 'html', 'gmi', 'md', 'js', 'css', 'noun']);

// How a page published under mark renders, as lattice's reader does:
// 'html' (only ever in a sandbox), 'md', 'gmi' or 'text' (in a <pre>).
// Only %gmi is sniffed, since %gmi is all an older publisher, an old
// revision or the vault peek says; no mark (a /k/ or /t/ read) is %gmi.
export function renderAs(mark, body) {
  if (mark === undefined || mark === null || mark === 'gmi') return looksHtml(body) ? 'html' : 'gmi';
  return mark === 'html' || mark === 'md' ? mark : 'text';
}

// ------------------------------------------------------------- stored rows

// rows as [{key, val}], val null for a deleted row
export function readRows(buf) {
  const v = new DataView(buf.buffer, buf.byteOffset, buf.byteLength), out = [];
  for (let at = 0; at + 8 <= buf.length;) {
    const kz = v.getUint32(at, true);
    const key = buf.slice(at + 4, at + 4 + kz);
    const vz = v.getUint32(at + 4 + kz, true);
    const val = vz === GONE ? null : buf.slice(at + 8 + kz, at + 8 + kz + vz);
    out.push({ key, val });
    at += 8 + kz + (val ? vz : 0);
  }
  return out;
}

export function writeRows(rows) {
  const out = new Uint8Array(rows.reduce((n, r) => n + 8 + r.key.length + (r.val ? r.val.length : 0), 0));
  const v = new DataView(out.buffer);
  let at = 0;
  for (const { key, val } of rows) {
    v.setUint32(at, key.length, true);
    out.set(key, at + 4);
    v.setUint32(at + 4 + key.length, val ? val.length : GONE, true);
    if (val) out.set(val, at + 8 + key.length);
    at += 8 + key.length + (val ? val.length : 0);
  }
  return out;
}

// ------------------------------------------------------------- hostile nouns

// Every noun another ship sends us is hostile (docs/rust-core.md, rule
// 4 of the port, on the rust-core branch). Each one is read under these
// caps, and anything past one is an UrbError:
//
//   MAX_CELLS   200,000 cells in one noun (a poke, a keen's answer, a
//               nack), counted as a tree: a cell reached twice counts
//               twice, as lattice's +small-noun counts (lattice-pub.hoon,
//               which bounds a peer's /pub/revs the same). So shared
//               structure, a small message that is vast as a tree, is
//               refused, and no decoder below walks more than this.
//   MAX_DEPTH   64: a $json nested deeper is refused
//   TANK_DEPTH  32: a tank nested deeper prints as nothing
//   MAX_NUMBER  32 bytes: an atom read as a number (a hash, an id, a
//               date, a count, a flag) is at most 256 bits; a ship 128,
//               and a %snap's snap-id 64, since grubbery makes it by
//               (shaz eny), an @uvJ of 512 bits
//   MAX_SEGS    64 segments in a path, each at most MAX_KNOT, 256 bytes,
//               as libmesa caps them (PATH_MAX_SEGS, KNOT_MAX)
//   MAX_BYTES   64 MiB of atoms in one noun, counted as a tree, as
//               MAX_CELLS is: one big atom a jam backrefs at each of its
//               places counts at each. 64 MiB is the most libmesa takes
//               in one message (ENG_MAX_MSG), so no noun a peer sends
//               unshared is over it. A decode that reads a noun more than
//               once (a tank's separator, printed between each two kids,
//               or a directory's files, a tree read at each place it is
//               named) counts what it reads, or prints, at each reading
//               against the same caps.
//   shape       an atom where a cell goes, a cell where an atom goes, or
//               a tag we don't know, is refused
//
// Nothing hostile traps the module: libmesa aborts on the head of an
// atom, so Noun checks first. A peek, keen or poke waiting on the noun
// fails with the error; one nobody waits on is logged and dropped.
// furum.mjs adds its own cap, on the posts a board may claim.
export const MAX_CELLS = 200_000;
export const MAX_DEPTH = 64;
export const TANK_DEPTH = 32;
export const MAX_NUMBER = 32;
export const MAX_SEGS = 64;
export const MAX_KNOT = 256;
export const MAX_BYTES = 64 << 20;

const hostile = what => new UrbError(`another ship's noun ${what}`);

// what one decode may still read: cells and atom bytes, counted as a
// tree, at each place they are read, and what it prints past them
const budget = () => ({ cells: MAX_CELLS, bytes: MAX_BYTES });
const MIB = `${MAX_BYTES >> 20} MiB`;
const spend = (b, bytes) => {
  if ((b.bytes -= bytes) < 0) throw hostile(`has over ${MIB} of atoms`);
};
// output a decode makes beyond the atoms it read: a tank's separators,
// a directory's paths
const print = (b, bytes) => {
  if ((b.bytes -= bytes) < 0) throw hostile(`reads as over ${MIB}`);
};

// n, once it is known to have at most MAX_CELLS cells and MAX_BYTES
// bytes of atoms as a tree, spent from b
function measure(n, b = budget()) {
  for (const todo = [n]; todo.length;) {
    const t = todo.pop();
    if (!t.isCell) {
      spend(b, t.size);
      continue;
    }
    if (--b.cells < 0) throw hostile(`has over ${MAX_CELLS} cells`);
    todo.push(t.tail, t.head);
  }
  return n;
}

// Each decoder bounds its own work too, for a noun that wasn't measured:
// a list or a map of over MAX_CELLS items, which shared structure can
// fake in a small noun, is refused.
function* list(n) {
  for (let i = 0; n.isCell; n = n.tail) {
    if (++i > MAX_CELLS) throw hostile(`has a list over ${MAX_CELLS} long`);
    yield n.head;
  }
}

// the nodes of a hoon map (a treap of [n l r])
function entries(n) {
  const out = [], stack = [n];
  while (stack.length) {
    const t = stack.pop();
    if (t.isCell) {
      if (out.length >= MAX_CELLS) throw hostile(`has a map over ${MAX_CELLS} entries`);
      out.push(t.head);
      stack.push(t.tail.head, t.tail.tail);
    }
  }
  return out;
}

const unit = n => (n.isCell ? n.tail : null);
const cord = n => (n.isCell ? null : n.text());

// a path's segments
function segments(p) {
  const out = [];
  for (const s of list(p)) {
    if (out.length >= MAX_SEGS) throw hostile(`has a path over ${MAX_SEGS} segments`);
    if (s.size > MAX_KNOT) throw hostile(`has a path segment over ${MAX_KNOT} bytes`);
    out.push(s.text());
  }
  return out;
}
const pathOf = p => '/' + segments(p).join('/');
const knot = s => {
  if (s.size > MAX_KNOT) throw hostile(`has a path segment over ${MAX_KNOT} bytes`);
  return s.text();
};

// a $json noun as JS
function json(n) {
  measure(n);
  const at = (n, depth) => {
    if (!n.isCell) return null;
    if (depth >= MAX_DEPTH) throw hostile(`has $json nested over ${MAX_DEPTH} deep`);
    const tag = n.head.text(), p = n.tail;
    if (tag === 'o') return Object.fromEntries(entries(p).map(kv => [kv.head.text(), at(kv.tail, depth + 1)]));
    if (tag === 'a') return [...list(p)].map(v => at(v, depth + 1));
    if (tag === 'b') return p.int() === 0n;
    if (tag === 'n') return Number(p.text());
    if (tag === 's') return p.text();
    throw hostile('has $json of no kind we know');
  };
  return at(n, 0);
}

// the text of a tank, as +ram flattens it; '' past TANK_DEPTH
function tape(t) {
  return dec.decode(Uint8Array.from([...list(t)].map(c => (c.isCell || c.size > MAX_NUMBER ? 63 : Number(c.int() & 0xffn)))));
}
// A rose's separator is printed between each two kids, so a long one
// over many kids prints far more than the noun holds: what it prints
// past the first is spent from b, a byte a character.
function tankText(t, b, depth = 0) {
  if (depth > TANK_DEPTH || !t.isCell || t.head.isCell) return '';
  const tag = t.head.text();
  if (tag === 'leaf') return tape(t.tail);
  if (tag !== 'rose' && tag !== 'palm') return '';
  const pre = t.tail.head;
  const [sep, opn, cls] = tag === 'rose'
    ? [tape(pre.head), tape(pre.tail.head), tape(pre.tail.tail)]
    : [tape(pre.head), tape(pre.tail.head) + tape(pre.tail.tail.head), tape(pre.tail.tail.tail)];
  const kids = [...list(t.tail.tail)];
  if (kids.length > 1) print(b, (kids.length - 1) * [...list(pre.head)].length);
  return opn + kids.map(k => tankText(k, b, depth + 1)).join(sep) + cls;
}

// the lines of a tang, whose caller measured it, from b if it measured
// it with one
const tangText = (t, b = budget()) => [...list(t)].map(k => tankText(k, b)).join('\n  ');

// the lines of an error noun [tag tang] (a nack's naxplanation), or what
// was wrong with it
function tang(err) {
  try {
    const b = budget();
    measure(err, b);
    if (!err.isCell) return '';
    return [err.head.isCell ? '' : err.head.text(), ...[...list(err.tail)].map(t => tankText(t, b))].join('\n  ');
  } catch (e) {
    if (e instanceof UrbError) return e.message;
    throw e;
  }
}

// the file a %snap's pace names, out of its inline or %data silo
function fileOf(snap, silo, conv) {
  const pace = snap.tail.head;
  const jobe = ['firm', 'temp'].includes(pace.head.text()) ? unit(pace.tail) : null;
  if (!jobe) return null;
  const want = jobe.int();
  const ject = entries(silo.tail).find(e => e.head.int() === want)?.tail.tail;
  if (!ject || ject.head.text() !== 'leaf') return null;
  const key = ject.tail.head.int();
  const val = entries(silo.head).find(e => e.head.int() === key)?.tail.tail;
  return val ? conv(val) : null;   // convert now: nouns are borrowed
}

// The files of a directory's %snap, deep: [[path, noun]], each path
// relative to it, name last; null if the silo lacks its tree. A tree
// ject is [%tree nek gain bang fil=(map @ta jobe) dir=(map @ta [jobe
// weir])]. Jects are named by their hashes, so two directories with the
// same contents are one ject, read at each place it is, as lattice's
// ball holds both. A tree that holds itself, which hashes can fake, is
// refused, and so is one deeper than MAX_SEGS, or a walk past MAX_CELLS
// trees and their entries, files and directories (shared trees fake a
// vast one). A name that is not a leaf where a file goes, or not a tree
// where a directory goes, is skipped, as one missing from the silo is.
// Each file's noun and its path, as /-separated text, are spent from
// one budget at each place the file is: one noun named by many files, or
// one tree at many places, counts at each, and so does each segment of
// a path, even an empty one. So a ball over 64 MiB as files is refused,
// even if its message is smaller. conv takes the list.
function ballOf(snap, silo, conv) {
  const pace = snap.tail.head;
  const top = ['firm', 'temp'].includes(pace.head.text()) ? unit(pace.tail) : null;
  if (!top) return null;
  const jects = new Map(entries(silo.tail).map(e => [e.head.int(), e.tail.tail]));
  const nouns = new Map(entries(silo.head).map(e => [e.head.int(), e.tail.tail]));
  const out = [], above = new Set(), b = budget();
  let work = 0;
  const count = () => {
    if (++work > MAX_CELLS) throw hostile(`has a directory of over ${MAX_CELLS} trees and files`);
  };
  //  at: the path's segments; len: its bytes as text, a / for each
  const walk = (jobe, at, len) => {
    const ject = jects.get(jobe);
    if (!ject || ject.head.text() !== 'tree') return false;
    if (above.has(jobe)) throw hostile('has a directory that holds itself');
    if (at.length > MAX_SEGS) throw hostile(`has a directory over ${MAX_SEGS} deep`);
    count();
    above.add(jobe);
    const fil = ject.tail.tail.tail.tail.head, dir = ject.tail.tail.tail.tail.tail;
    for (const e of entries(fil)) {
      count();
      const leaf = jects.get(e.tail.int());
      if (!leaf || leaf.head.text() !== 'leaf') continue;
      const val = nouns.get(leaf.tail.head.int());
      if (!val) continue;
      const name = knot(e.head);
      print(b, len + 1 + e.head.size);
      out.push([[...at, name], measure(val, b)]);
    }
    for (const e of entries(dir)) {
      count();
      walk(e.tail.head.int(), [...at, knot(e.head)], len + 1 + e.head.size);
    }
    above.delete(jobe);
    return true;
  };
  if (!walk(top.int(), [], 0)) return null;
  return conv(out);   // convert now: nouns are borrowed
}

// the decoders of other ships' nouns, for tests: each takes any noun
// with isCell, head, tail, size, bytes(), text() and int()
export const decoders = { measure, list, entries, unit, cord, segments, pathOf, json, tang, tangText, fileOf, ballOf };

// ------------------------------------------------------------- messages

// a path noun from "/a/b/c", keeping empty segments as stab does
export function path(text) {
  const t = text.replace(/^\//, '');
  return t ? [...t.split('/'), null] : null;
}

// A file's version, as grubbery's cass:clay: [ud=rev da=at]
export const cass = f => [BigInt(f.rev), da(f.at)];

// A file's wave, the version news grubbery sends whoever keeps it: here
// its folder's [fold-cass {name: cass}] with the one file in it, which is
// all lattice's page subscription reads (+sub-apply-wave). Empty for a
// file that never was.
export const waveOf = (f, name) => (f?.rev ? [[null, cass(f), new Map([[name, cass(f)]])], null] : [null, null]);

// A file as grubbery's %snap: its version, and the content-addressed
// silo that holds it, sent inline. The reader checks that each noun and
// ject hashes (+sham) to its name, then reads the noun and its mark.
// sham is the engine's (nouns.sham).
// ponytail: always inline, never the %want leg grubbery uses past 8 KB,
// so a file rides whole in one message.
export function snapOf(f, sham) {
  const nobe = sham(f.value);
  const ject = ['leaf', nobe, [[path(f.mark[0]), f.mark[1]], 0, null], true, null];
  const jobe = sham(ject);
  return [
    [cass(f), ['firm', null, jobe], [new Set([jobe]), new Set([nobe])]],
    [new Map([[nobe, [0, f.value]]]), new Map([[jobe, [0, ject]]])],
  ];
}

// The messages we send other ships' grubbery, as values for noun() (the
// molds are grubbery's load:remo, transfer:remo and intake:remo, which
// check.mjs clams them against). A wire given as text is a path; one
// that came in a message we answer goes back as it came, a noun.
export const messages = {
  // %grubbery-load: a peek of [folder name], or with name null of the
  // directory folder and all under it
  peek: (wire, folder, name) =>
    [[path(wire), name === null ? [false, path(folder)] : [true, path(folder), name]], ['peek', null, true]],
  // %grubbery-load: [mark value] to the grub [folder name], mark a blot
  poke: (wire, folder, name, mark, value) =>
    [[path(wire), [true, path(folder), name]], ['poke', [path(mark[0]), mark[1]], value]],
  // %grubbery-transfer: a poke taken in
  pack: wire => [wire, 'pack', null],
  // %grubbery-transfer: the content of a %snap that came without it
  want: (wire, dest, id) => [wire, 'want', dest, id],
  // %grubbery-transfer: our answer to a peek, snap and data null for none
  snap: (wire, dest, id, snap, data) => [wire, 'snap', dest, id, snap && [null, snap], data && [null, data]],
  // %grubbery-intake: a keep's wave
  intake: (wire, dest, wave) => [wire, dest, wave],
};

// ------------------------------------------------------------- the relay

// Datagrams over a WebSocket to `urb-fetch -relay`, which sends and
// receives them on a UDP port it keeps for this ship. The first message
// names the ship; then each frame out is [kind:1][galaxy:1][ip:4][port:2]
// [datagram] (kind 1: to that galaxy, which the relay looks up), and each
// frame in is [ip:4][port:2][datagram], big-endian. While the socket is
// down datagrams are dropped, and the engine resends them.
export function relayTransport(url, log = console.error) {
  let ws = null, closed = false, ship, onPacket;
  const connect = () => new Promise((resolve, reject) => {
    const s = new WebSocket(url);
    let opened = false;
    s.binaryType = 'arraybuffer';
    s.onopen = () => {
      opened = true;
      ws = s;
      s.send(ship);
      resolve();
    };
    s.onmessage = e => {
      if (typeof e.data === 'string' || e.data.byteLength < 6) return;
      const v = new DataView(e.data);
      onPacket(v.getUint32(0), v.getUint16(4), new Uint8Array(e.data, 6));
    };
    s.onerror = () => {};
    s.onclose = () => {
      if (ws === s) ws = null;
      if (!opened) reject(new Error(`can't reach the relay at ${url}`));
      if (!closed) setTimeout(() => connect().catch(() => {}), 2000);
    };
  });
  return {
    start(s, f) {
      ship = s;
      onPacket = f;
      return connect().catch(e => {
        closed = true;
        throw e;
      });
    },
    send(lane, bytes) {
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      const f = new Uint8Array(8 + bytes.length);
      const v = new DataView(f.buffer);
      if (lane.name) {
        v.setUint8(0, LANE_GALAXY);
        v.setUint8(1, lane.galaxy);
      } else {
        v.setUint8(0, LANE_IP);
        v.setUint32(2, lane.ip);
        v.setUint16(6, lane.port);
      }
      f.set(bytes, 8);
      ws.send(f);
    },
    close() {
      closed = true;
      ws?.close();
    },
  };
}

// ------------------------------------------------------------- the engine

const randomBytes = n => crypto.getRandomValues(new Uint8Array(n));
const globalTimers = { setTimeout: (f, ms) => globalThis.setTimeout(f, ms), clearTimeout: t => globalThis.clearTimeout(t) };
const globalFetch = (...a) => globalThis.fetch(...a);

export async function open({ wasm, transport, store, log = console.error, env = {}, fake = false,
                             now = Date.now, entropy = randomBytes,
                             timers = globalTimers, fetch = globalFetch }) {
  const { setTimeout, clearTimeout } = timers;
  let X;
  const mem = () => new Uint8Array(X.memory.buffer);   // replaced when memory grows
  const imports = {
    wasi_snapshot_preview1: wasi(() => X.memory.buffer, log, env),
    host: guarded({ send, need_point, on_poke, on_done, on_sage, on_peek, store_commit }, log),
  };
  const inst = wasm instanceof WebAssembly.Module
    ? await WebAssembly.instantiate(wasm, imports)
    : (await WebAssembly.instantiate(wasm, imports)).instance;
  X = inst.exports;
  X._initialize();

  // ----------------------------------------------------------- memory

  function put(bytes) {
    const p = X.alloc(bytes.length || 1);
    mem().set(bytes, p);
    return p;
  }

  function withBytes(bytes, f) {
    const p = put(bytes);
    try { return f(p, bytes.length); } finally { X.free(p); }
  }

  const withStr = (s, f) => withBytes(enc.encode(s + '\0'), f);

  function cstr(p) {
    const m = mem();
    let e = p;
    while (m[e]) e++;
    return dec.decode(m.subarray(p, e));
  }

  // a string libmesa malloc'd
  function take(p) {
    if (!p) return null;
    const s = cstr(p);
    X.free(p);
    return s;
  }

  function check(err, what) {
    if (err !== 0) throw new Error(`${what} failed: mesa error ${err}`);
  }

  // a ship is [lo, hi], two BigInt halves
  function twoU64(f) {
    const out = X.alloc(16);
    try {
      if (!f(out)) return null;
      const v = new DataView(X.memory.buffer, out, 16);
      return [v.getBigUint64(0, true), v.getBigUint64(8, true)];
    } finally {
      X.free(out);
    }
  }

  function shipOf(text) {
    const s = withStr(text, p => twoU64(out => X.ship_parse(p, out)));
    if (!s) throw new UrbError(`not a ship: ${text}`);
    return s;
  }

  const shipText = (lo, hi) => take(X.ship_render(lo, hi));

  // ----------------------------------------------------------- nouns

  // a borrowed noun handle, valid during the hook that handed it out.
  // Its shape is checked before libmesa sees it: libmesa aborts on the head
  // of an atom or the bytes of a cell, and nouns from other ships can be
  // any shape.
  class Noun {
    constructor(n) { this.n = n; }
    get isCell() { return X.is_cell(this.n) !== 0; }
    get head() { return new Noun(X.head(this.cell())); }
    get tail() { return new Noun(X.tail(this.cell())); }
    cell() {
      if (!this.isCell) throw hostile('has an atom where a cell goes');
      return this.n;
    }
    // an atom's length in bytes
    get size() {
      if (this.isCell) throw hostile('has a cell where an atom goes');
      return X.met3(this.n);
    }
    bytes() {
      const len = this.size;
      const p = X.alloc(len || 1);
      try {
        X.atom_bytes(this.n, p, len);
        return mem().slice(p, p + len);
      } finally {
        X.free(p);
      }
    }
    text() { return dec.decode(this.bytes()); }
    // as a BigInt, at most max bytes of it
    int(max = MAX_NUMBER) {
      if (this.size > max) throw hostile(`has a number over ${max} bytes`);
      const b = this.bytes();
      let v = 0n;
      for (let i = b.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(b[i]);
      return v;
    }
  }

  // a new noun (an owned reference) from a JS value: null (~), a boolean
  // (a loobean), a number or BigInt, a string (a cord), a Noun, an array
  // of two or more, right-associated into cells, or a Map or Set, a hoon
  // map or set of those
  function noun(x) {
    if (x === null) return atom(new Uint8Array());
    if (typeof x === 'boolean') return atom(new Uint8Array(x ? [] : [1]));
    if (typeof x === 'number' || typeof x === 'bigint') {
      const b = [];
      for (let v = BigInt(x); v > 0n; v >>= 8n) b.push(Number(v & 0xffn));
      return atom(Uint8Array.from(b));
    }
    if (typeof x === 'string') return atom(enc.encode(x));
    if (x instanceof Noun) return X.ref(x.n);
    if (Array.isArray(x) && x.length >= 2) {
      let n = noun(x.at(-1));
      for (let i = x.length - 2; i >= 0; i--) n = X.cell(noun(x[i]), n);
      return n;
    }
    if (x instanceof Map || x instanceof Set) return treap(x);
    throw new TypeError(`can't make a noun of ${x}`);
  }

  const atom = bytes => withBytes(bytes, (p, n) => X.atom(p, n));
  const mug = n => X.mug(n) >>> 0;

  // +dor, depth-first order: the tiebreak when two mugs are equal
  function dor(a, b) {
    if (X.eq(a, b)) return true;
    if (X.is_cell(a)) {
      if (!X.is_cell(b)) return false;
      return X.eq(X.head(a), X.head(b)) ? dor(X.tail(a), X.tail(b)) : dor(X.head(a), X.head(b));
    }
    return X.is_cell(b) || new Noun(a).int(Infinity) < new Noun(b).int(Infinity);
  }

  // A hoon map (of a Map) or set (of a Set) as +put:by and +put:in build
  // it: a treap, keys in +gor order (by mug) and the +mor-first key (by
  // the mug of the mug) on top. A map is validated against exactly this
  // shape when it is clammed, and its keys fix it, so it is built at once.
  // ponytail: O(n^2) in the keys, fine for a few hundred.
  function treap(x) {
    const isMap = x instanceof Map;
    const items = [...x].map(e => {
      const key = noun(isMap ? e[0] : e), m = mug(key), mm = noun(m);
      const it = { key, val: isMap ? e[1] : undefined, m, mm: mug(mm) };
      X.unref(mm);
      return it;
    });
    const before = (a, b, k) => (a[k] !== b[k] ? a[k] < b[k] : dor(a.key, b.key));
    const build = list => {
      if (!list.length) return noun(null);
      const top = list.reduce((a, b) => (before(b, a, 'mm') ? b : a));
      const rest = list.filter(e => e !== top);
      const left = rest.filter(e => before(e, top, 'm')), right = rest.filter(e => !left.includes(e));
      const [l, r] = [build(left), build(right)];
      return X.cell(isMap ? X.cell(top.key, noun(top.val)) : top.key, X.cell(l, r));
    };
    return build(items);
  }

  // +sham of a value (as noun takes it): the hash grubbery names content by
  function sham(x) {
    const n = noun(x), out = X.alloc(16);
    try {
      X.sham(n, out);
      return bigOf(mem().slice(out, out + 16));
    } finally {
      X.free(out);
      X.unref(n);
    }
  }

  // ----------------------------------------------------------- the loop

  let timer = null;

  // after every engine call: wake the engine at its next deadline
  function arm() {
    clearTimeout(timer);
    const d = u64(X.deadline());
    if (d === NONE) return;
    //  setTimeout can't wait longer than 2^31 ms; waking early just re-arms
    const wait = Math.min(Math.max(0, Number(d) - now()), 60_000);
    timer = setTimeout(() => {
      if (now() >= Number(u64(X.deadline()))) call(() => X.wake(BigInt(now())));
      else arm();
    }, wait, 'wake');
  }

  function call(f) {
    try { return f(); } finally { arm(); }
  }

  // engine calls that commit together
  function txn(f) {
    return call(() => {
      check(X.txn_begin(), 'txn_begin');
      try {
        const r = f();
        check(X.txn_commit(), 'txn_commit');
        return r;
      } catch (e) {
        X.txn_abort();
        throw e;
      }
    });
  }

  // each commit's rows are saved before the datagrams it released go out
  let saved = Promise.resolve();

  // hook: save what a commit changed
  function store_commit(p, len) {
    const rows = mem().slice(p, p + len);
    saved = saved.then(() => store.commit(rows)).catch(e => log(`saving the engine's state failed: ${e.message}`));
    return 0;
  }

  // galaxy number -> name, for the transport to look up
  const galaxyNames = new Map();

  // hook: send one datagram. Copy it out now: memory can grow before it goes
  function send(kind, gal, ip, port, ptr, len) {
    const bytes = mem().slice(ptr, ptr + len);
    let lane = null;
    if (kind === LANE_GALAXY) {
      if (!galaxyNames.has(gal)) galaxyNames.set(gal, shipText(BigInt(gal), 0n).slice(1));
      lane = { galaxy: gal, name: galaxyNames.get(gal) };
    } else if (kind === LANE_IP && ip && port) {
      lane = { ip: ip >>> 0, port };
    }
    if (lane) saved.then(() => transport.send(lane, bytes));
  }

  // hook: find a ship's keys on azimuth; answer once the roller has
  const asking = new Set();

  function need_point(lo, hi) {
    [lo, hi] = [u64(lo), u64(hi)];
    //  a fake network's keys come from names: answer once the engine call
    //  that asked has returned
    if (fake) return void setTimeout(() => call(() => X.point_fake(lo, hi)), undefined, `point ${shipText(lo, hi)}`);
    const k = `${lo}:${hi}`;
    if (asking.has(k)) return;
    asking.add(k);
    const body = take(X.point_request(lo, hi));
    //  given up on after 30 s, by a timer of the engine's (so a trace has it)
    const ac = new AbortController();
    const t = setTimeout(() => ac.abort(new DOMException('The operation was aborted due to timeout', 'TimeoutError')),
                         30_000, `roller ${shipText(lo, hi)}`);
    fetch(ROLLER, { method: 'POST', headers: { 'content-type': 'application/json' }, body, signal: ac.signal })
      .then(r => r.text())
      .finally(() => clearTimeout(t))
      .catch(e => (log(`azimuth lookup of ${shipText(lo, hi)} failed: ${e.message}`), null))
      .then(js => {
        asking.delete(k);
        call(() => (js === null ? X.point(lo, hi, 0, 0)
                                : withBytes(enc.encode(js), (p, n) => X.point(lo, hi, p, n))));
      });
  }

  function onPacket(ip, port, bytes) {
    withBytes(bytes, (p, n) => call(() => X.hear(ip, port, p, n, BigInt(now()))));
  }

  // ----------------------------------------------------------- the reader

  const peeks = new Map();   // wire -> state of one peek
  const pokes = new Map();   // wire -> state of one grubbery poke
  const poking = new Map();  // bone -> that poke's state: each has a flow of its own
  let inbox = null;          // who hears grubbery pokes to us, see grub.onInbox
  const keens = new Map();   // scry path -> {done, promise}
  const flows = new Map();   // ship -> bone of our flow to its %grubbery
  const bases = new Map();   // ship -> where it runs lattice
  const revs = new Map();    // ship -> {at, list}
  //  wires never repeat across runs: a publisher can deliver a reply meant
  //  for an earlier run late, and it must not match ours
  const nonce = [...entropy(4)].map(b => b.toString(16).padStart(2, '0')).join('');
  let wires = 0;

  function poke(her, bone, agent, mark, value) {
    const n = noun(value);
    withStr(agent, a => withStr(mark, m => check(X.poke(her[0], her[1], bone, a, m, n), 'poke')));
  }

  // our one flow to [her]'s %grubbery, for peeks and answers
  function flowTo(her) {
    const k = `${her[0]}:${her[1]}`;
    if (!flows.has(k)) {
      const bone = u64(X.flow_open(her[0], her[1]));
      if (bone === NONE) throw new Error('flow_open failed');
      flows.set(k, bone);
    }
    return flows.get(k);
  }

  // grubbery's cross-ship peek of one file: [dest-dir, name], or with
  // name null of the directory folder and all under it (a deep peek, as
  // lattice's +peek-remote-wait). conv takes the file's noun, or the
  // directory's files as [[path, noun]]. Null after wait ms.
  function peek(her, folder, name, conv, wait = PEEK_TIMEOUT) {
    const wire = `/urb-fetch/${nonce}/${++wires}`;
    return new Promise((resolve, reject) => {
      const st = { read: name === null ? (snap, silo) => ballOf(snap, silo, conv) : (snap, silo) => fileOf(snap, silo, conv),
                   snap: null, done: false, her: `${her[0]}:${her[1]}` };
      const settle = f => {
        if (st.done) return;
        st.done = true;
        clearTimeout(st.timer);
        peeks.delete(wire);
        if (st.snap) X.unref(st.snap.n);
        f();
      };
      st.finish = v => settle(() => resolve(v));   // null: refused, nothing there, or no answer
      st.fail = e => settle(() => reject(e));      // an answer we could not read
      st.timer = setTimeout(() => st.finish(null), wait, `peek ${wire}`);
      peeks.set(wire, st);
      txn(() => {
        st.bone = flowTo(her);
        poke(her, st.bone, 'grubbery', 'grubbery-load', messages.peek(wire, folder, name));
      });
    });
  }

  // grubbery's cross-ship poke: [mark noun] to the grub [folder name] on
  // [her], where mark is [path name] (a blot, e.g. ['/furum', 'msg']).
  // Resolves {ok} once the grub took it in, or {ok: false, error}: refused
  // (a nack), or no answer in time (with later). Its own flow, so a nack or
  // an ack on that flow is this poke's, and the flow is corked after.
  function grubPoke(her, folder, name, mark, value) {
    const wire = `/grub-poke/${nonce}/${++wires}`;
    return new Promise(resolve => {
      const st = { done: false, her };
      st.finish = r => {
        if (st.done) return;
        st.done = true;
        clearTimeout(st.timer);
        pokes.delete(wire);
        resolve(r);
      };
      st.timer = setTimeout(() => st.finish({ ok: false, error: `${shipText(...her)} did not answer`, later: true }), PEEK_TIMEOUT, `poke ${wire}`);
      pokes.set(wire, st);
      txn(() => {
        const bone = u64(X.flow_open(her[0], her[1]));
        if (bone === NONE) throw new Error('flow_open failed');
        st.bone = bone;
        poking.set(bone, st);
        poke(her, bone, 'grubbery', 'grubbery-load', messages.poke(wire, folder, name, mark, value));
      });
    });
  }

  // hook: a poke to us: %grubbery-transfer answers our peeks and pokes, and
  // a %grubbery-load is another ship's grubbery reading our files (%peek,
  // %keep, %drop) or a grub there writing to ours (%poke)
  function on_poke(lo, hi, bone, agentPtr, markN, nounN) {
    [lo, hi, bone] = [u64(lo), u64(hi), u64(bone)];
    if (cstr(agentPtr) !== 'grubbery') return ACK;
    const mark = new Noun(markN).text();
    const n = new Noun(nounN);
    if (mark === 'grubbery-load') {
      //  nobody waits on it: one we can't read is logged (by guarded)
      measure(n);
      //  [[wire dest] %poke [blot noun]], dest a rail [%& [path name]]
      const dest = n.head.tail, act = n.tail, tag = act.head.text();
      if (['peek', 'keep', 'drop'].includes(tag)) {
        serveLoad([lo, hi], n.head.head, dest, tag);
        return ACK;
      }
      if (tag !== 'poke') return ACK;
      if (inbox && !dest.head.int()) {
        const blot = act.tail.head;
        const msg = {
          from: shipText(lo, hi),
          to: [pathOf(dest.tail.head), dest.tail.tail.text()],
          mark: [pathOf(blot.head), blot.tail.text()],
          noun: act.tail.tail,
        };
        try { inbox(msg); } catch (e) { log(e); }
      }
      //  say it was taken in: the sender waits for this %pack, on its wire
      poke([lo, hi], flowTo([lo, hi]), 'grubbery', 'grubbery-transfer', messages.pack(n.head.head));
      return ACK;
    }
    if (mark !== 'grubbery-transfer') return ACK;
    const wire = pathOf(n.head), from = `${lo}:${hi}`;
    try {
      const b = budget();
      measure(n, b);
      transfer(wire, from, n.tail.head.text(), n.tail.tail, [lo, hi], n.head, b);
    } catch (e) {
      //  what waits on this wire fails with it: for /want, every %want
      //  out to the ship that sent it
      const p = pokes.get(wire), st = peeks.get(wire);
      const wants = wire === '/want' ? [...peeks.values()].filter(w => w.snap && !w.done && w.her === from) : [];
      if (p) p.finish({ ok: false, error: e.message });
      else if (st) st.fail(e);
      else if (wants.length) for (const w of wants) w.fail(e);
      else log(`${shipText(lo, hi)}: ${e.message}`);
    }
    return ACK;
  }

  // a %grubbery-transfer [wire tag r] from [from] (as lo:hi), measured
  // from b
  function transfer(wire, from, tag, r, her, wireN, b) {
    if (pokes.has(wire)) {
      //  [%pack err=(unit tang)]: the grub took the poke in, or refused it
      const err = tag === 'pack' ? unit(r) : null;
      pokes.get(wire).finish(err ? { ok: false, error: tangText(err, b) } : { ok: tag === 'pack' });
      return;
    }
    if (wire === '/want') {
      //  +process-want answers on the fixed wire /want, not ours, so match
      //  the content to the snap that asked for it, of the ship that sent it
      const wants = [...peeks.values()].filter(st => st.snap && !st.done && st.her === from);
      for (const st of wants) {
        let v;
        try {
          v = tag === 'data' ? st.read(st.snap, r) : null;
        } catch (e) {
          st.fail(e);
          continue;
        }
        if (v !== null) st.finish(v);
      }
      if (tag === 'miss' && wants.length) wants[0].finish(null);   // the pin expired
      return;
    }
    const st = peeks.get(wire);
    if (!st) return;
    if (tag === 'snap') {
      //  [dest snap-id snap=(unit [cass pace refs]) data=(unit silo)]
      const snap = unit(r.tail.tail.head), data = unit(r.tail.tail.tail);
      if (snap && !data) {
        //  too big to inline: ask for the content, pinned for 5 minutes
        const id = r.tail.head.int(64);   // an @uvJ, (shaz eny): 512 bits
        st.snap = new Noun(X.ref(snap.n));
        poke(her, st.bone, 'grubbery', 'grubbery-transfer', messages.want(wireN, r.head, id));
        return;
      }
      st.finish(snap ? st.read(snap, data) : null);
      return;
    }
    st.finish(null);   // %veto, %miss
  }

  // hook: our poke was acked, or nacked
  function on_done(lo, hi, bone, error) {
    [lo, hi, bone, error] = [u64(lo), u64(hi), u64(bone), u64(error)];
    const st = poking.get(bone);
    if (st) {
      //  a grubbery poke's own flow: closed once answered
      poking.delete(bone);
      X.flow_cork(lo, hi, bone);
      if (error !== NONE) st.finish({ ok: false, error: tang(new Noun(error)) });
      return;
    }
    if (error === NONE) return;
    log(`nack from ${shipText(lo, hi)}: ${tang(new Noun(error))}`);
    for (const st of [...peeks.values()]) if (st.bone === bone) st.finish(null);
  }

  // one keen at a time per path; readers asking the same share it
  function keen(her, spath) {
    if (keens.has(spath)) return keens.get(spath).promise;
    const k = {};
    k.promise = new Promise((resolve, reject) => {
      const t = setTimeout(() => {
        //  a miss never answers: stop asking, as +keen-page-raw %yawns
        keens.delete(spath);
        txn(() => withStr(spath, p => X.yawn(her[0], her[1], p)));
        resolve(null);
      }, KEEN_TIMEOUT, `keen ${spath}`);
      k.done = v => {
        clearTimeout(t);
        keens.delete(spath);
        resolve(v);
      };
      k.fail = e => {
        clearTimeout(t);
        keens.delete(spath);
        reject(e);
      };
    });
    keens.set(spath, k);
    txn(() => check(withStr(spath, p => X.keen(her[0], her[1], p)), 'keen'));
    return k.promise;
  }

  // hook: the answer to a keen
  function on_sage(lo, hi, pathPtr, gage) {
    const k = keens.get(cstr(pathPtr));
    if (!k) return;
    if (u64(gage) === NONE) return void k.done(null);
    try {
      const g = measure(new Noun(gage));
      k.done({ mark: g.head.text(), body: cord(g.tail) });
    } catch (e) {
      k.fail(e);
    }
  }

  // ----------------------------------------------------------- serving

  // What we publish, as grub.serve and scry.serve set it: file(folder,
  // name) and scries(path), both null for anything we don't serve.
  let files = () => null;
  let scries = () => null;
  let keepers = new Map();   // `${ship} ${folder} ${name}` -> {ship, wire, folder, name}
  let keepersChanged = () => {};

  // A %peek, %keep or %drop of one of our files, in a hook: answer it on
  // our flow to the asker's %grubbery, as grubbery's +process-peek and
  // +keep do. Only files the host serves are ever answered with content.
  function serveLoad(her, wire, dest, tag) {
    const at = dest.head.int() ? null : [pathOf(dest.tail.head), dest.tail.tail.text()];
    const f = at && files(...at);
    const ship = shipText(...her), k = at && `${ship} ${at[0]} ${at[1]}`;
    if (tag === 'drop') {
      if (k && keepers.delete(k)) keepersChanged([...keepers.values()]);
      return;
    }
    if (tag === 'peek') {
      const [snap, data] = f && f.value !== undefined ? snapOf(f, sham) : [null, null];
      const id = bigOf(entropy(16));
      poke(her, flowTo(her), 'grubbery', 'grubbery-transfer', messages.snap(wire, dest, id, snap, data));
      return;
    }
    //  Only a file with something in it is kept, so one ship can't fill
    //  every slot with names we never published: a keep of one that isn't
    //  there (not yet, or no longer) gets its first wave and no news. A
    //  keep held already and unchanged isn't saved again, so a repeated
    //  keep costs no write.
    if (f && f.value !== undefined && (keepers.has(k) || keepers.size < MAX_KEEPERS)) {
      const kept = { ship, wire: segments(wire), folder: at[0], name: at[1] };
      if (JSON.stringify(keepers.get(k)) !== JSON.stringify(kept)) {
        keepers.set(k, kept);
        keepersChanged([...keepers.values()]);
      }
    }
    //  the first wave answers the keep, even for nothing
    poke(her, flowTo(her), 'grubbery', 'grubbery-intake', messages.intake(wire, dest, waveOf(f, at?.[1])));
  }

  // hook: a read of our namespace, a %keen from another ship
  function on_peek(pathPtr) {
    const got = scries(cstr(pathPtr));
    return got ? noun(got) : NONE;
  }

  // A peer's /pub/revs, or null if it has none there. One we can't read
  // (over the caps, or not a $json object) is an empty list, as lattice's
  // +fetch-peer-revs reads it, so its pages still read by the vault peek.
  const revsOf = n => {
    const j = json(n);
    return j && typeof j === 'object' && !Array.isArray(j) ? j : {};
  };
  const peekRevs = (her, b) => peek(her, b + '/pub', 'revs', revsOf)
    .catch(e => (e instanceof UrbError ? {} : Promise.reject(e)));

  // +peer-base: where the peer runs lattice, asked, not guessed
  async function base(her, who) {
    if (!bases.has(who)) {
      for (const b of [DESK_BASE, APP_BASE]) {
        const got = await peekRevs(her, b);
        if (got !== null) {
          bases.set(who, b);
          revs.set(who, { at: now(), list: got });
          break;
        }
      }
      //  later: a ship catching up on a comet that moved answers in a minute or two
      if (!bases.has(who)) throw Object.assign(new UrbError(`${who} answered no lattice peek (offline, or no lattice)`), { later: true });
    }
    return bases.get(who);
  }

  // +peer-revs-cached
  async function revList(her, who) {
    const b = await base(her, who);
    let r = revs.get(who);
    if (!r || now() - r.at > REVS_TTL) {
      r = { at: now(), list: (await peekRevs(her, b)) ?? {} };
      revs.set(who, r);
    }
    return r.list;
  }

  // +read-page-body-rev: keen the listed id, else peek the vault file.
  // [body, how, mark]. The vault file is %gmi whatever the page's format
  // and carries no other mark, so a page read by peek is %gmi.
  async function page(her, who, rel) {
    const key = '/' + rel.join('/');
    const ent = (await revList(her, who))[key];
    if (ent) {
      const got = await keen(her, `/g/x/1/grubbery//1/pub/page${key}/${ent.id}`);
      if (got && PAGE_MARKS.has(got.mark) && got.body !== null) return [got.body, 'keen', got.mark];
      revs.delete(who);   // a miss, or a %del tombstone: the list is stale
    }
    const body = await peek(her, (await base(her, who)) + '/pub/vault' + key, 'gmi', cord);
    return body !== null ? [body, 'peek', 'gmi'] : [null, null, null];
  }

  // a file in the publisher's own tree, which it shares with whom it
  // chooses: text, or a know entry's body
  async function readPrivate(her, who, url, folder, name) {
    const text = n => (n.isCell ? (n.head.isCell ? null : n.head.text()) : n.text());
    const body = await peek(her, folder, name, text);
    if (body === null) throw new UrbError(`${url}: nothing there, or not shared with ${our}`);
    return { key: `${folder}/${name}`, body, how: 'peek' };
  }

  // +read-page-or-index for one url
  async function read(url) {
    const [who, rel, mount] = deUrb(url);
    const her = shipOf(who);
    if (mount === 'p') throw new UrbError(`${url} is a live Lattice page, which only Lattice shows`);
    if (mount === 'forum') throw new UrbError(`${url} is a forum board: forum.mjs reads it`);
    if (mount === 'k') {
      if (!rel.length) throw new UrbError(`${url} names no know entry`);
      return readPrivate(her, who, url, `${await base(her, who)}/know/vault/${rel.join('/')}`, 'entry');
    }
    if (mount === 't') {
      if (!rel.length) throw new UrbError(`${url} is the publisher's whole tree, not a file`);
      return readPrivate(her, who, url, '/' + rel.slice(0, -1).join('/'), rel.at(-1));
    }
    const list = await revList(her, who);
    const hit = p => ('/' + p.join('/')) in list;
    const dex = [...rel, 'index'];
    const tries = !rel.length || rel.at(-1) === 'index' ? [rel]
      : !hit(rel) && hit(dex) ? [dex]
      : Object.keys(list).length && !hit(rel) ? [rel]
      : [rel, dex];
    for (const p of tries) {
      const [body, how, mark] = await page(her, who, p);
      if (body !== null) return { key: '/' + p.join('/'), body, how, mark };
    }
    throw new UrbError(`${url}: not published`);
  }

  // ----------------------------------------------------------- start

  let { keyfile, rows, fresh } = await store.load();
  if (!keyfile) {
    const res = await fetch(STARS, { headers: { 'user-agent': 'urb-fetch' } });   // 403 without one, outside a browser
    if (!res.ok) throw new Error(`${STARS}: ${res.status}`);
    const jam = new Uint8Array(await res.arrayBuffer());
    keyfile = withBytes(jam, (j, n) => withBytes(entropy(64), e => take(X.mine_comet(j, n, e))));
    if (!keyfile) throw new Error('mining a comet failed');
    await store.saveKey(keyfile);
    rows = null;
  }
  const us = withStr(keyfile, p => twoU64(out => X.keyfile_ship(p, out)));
  if (!us) throw new Error('the saved keyfile is not one');
  const our = shipText(...us);

  //  the transport is up before the engine opens: opening can send
  await transport.start(our, onPacket);
  if (rows?.length) check(withBytes(rows, (p, n) => X.store_load(p, n)), 'store_load');
  //  a fake ship is rebooted fresh, so it may always start from empty state
  const breached = fake || (fresh && !rows?.length);
  const err = call(() => withStr(keyfile, p => X.open(p, BigInt(now()), breached ? 1 : 0, fake ? 1 : 0)));
  if (err === -2) {
    transport.close();
    throw new Error(`${our} has run before, and its state here is empty. Starting it from nothing would replay old messages, so breach it on its parent (|moon-breach) and add its new key.`);
  }
  check(err, 'open');

  // the noun readers an app's own reading needs, valid during the call
  // that hands a noun out
  // a value as noun() builds it, as a plain tree: an atom a BigInt, a
  // cell [head, tail] (jam.mjs serializes it), for the conformance vectors
  function tree(x) {
    const walk = n => (X.is_cell(n) ? [walk(X.head(n)), walk(X.tail(n))] : new Noun(n).int(Infinity));
    const n = noun(x);
    try { return walk(n); } finally { X.unref(n); }
  }

  const nouns = {
    entries, list, unit, cord, json, sham, tree,
    mug: x => {
      const n = noun(x);
      try { return mug(n); } finally { X.unref(n); }
    },
    ship: n => {
      if (n.size > 16) throw hostile('has a ship over 128 bits');
      const b = n.bytes(), at = i => b.slice(i, i + 8).reduceRight((v, x) => (v << 8n) | BigInt(x), 0n);
      return shipText(at(0), at(8));
    },
  };

  return {
    our,
    read,
    nouns,
    // the clock and the entropy this engine was opened with, for pub.mjs
    now,
    entropy,
    // grubbery on other ships: read a file, poke a grub, and hear grubs
    // that poke ours (f({from, to, mark, noun}), during the hook)
    grub: {
      // name null peeks the directory folder, deep: conv([[path, noun]]),
      // each path relative to it, name last. wait is ms (the default 30 s)
      peek: (ship, folder, name, conv, wait) => peek(shipOf(ship), folder, name, conv, wait),
      poke: (ship, folder, name, mark, value) => grubPoke(shipOf(ship), folder, name, mark, value),
      onInbox: f => { inbox = f; },
      // Serve our files to other ships' grubbery: file(folder, name) gives
      // {mark: [path, name], value, rev, at} (at in ms; no value while there
      // is nothing there, not yet or no longer), or null for a file we don't
      // serve. Each keep of a file with something in it is kept until its
      // ship drops it: saved, the keeps the last run handed onKeepers,
      // which hears every change.
      serve(file, saved = [], onKeepers = () => {}) {
        files = file;
        keepers = new Map(saved.map(k => [`${k.ship} ${k.folder} ${k.name}`, k]));
        keepersChanged = onKeepers;
      },
      // tell whoever keeps [folder name] its new version
      changed(folder, name) {
        const f = files(folder, name);
        txn(() => {
          for (const k of keepers.values()) {
            if (k.folder !== folder || k.name !== name) continue;
            const her = shipOf(k.ship);
            poke(her, flowTo(her), 'grubbery', 'grubbery-intake',
                 messages.intake(k.wire.length ? [...k.wire, null] : null, [0, path(folder), name], waveOf(f, name)));
          }
        });
      },
    },
    // serve our scry namespace: f(path) gives [mark, value] or null, for
    // paths such as /g/x/1/grubbery//1/pub/page/<name>/<id>. A path must
    // never be bound to two different values.
    scry: { serve: f => { scries = f; } },
    // resolves once everything committed is saved and sent. The last acks
    // go out after a commit: a host that never gets them resends to us
    // later, and our next run waits behind those
    async close() {
      clearTimeout(timer);
      await saved;
      await new Promise(r => setTimeout(r, 500, 'close'));
      transport.close();
      X.close();
    },
  };
}

// What a keyfile is, without opening an engine: {ship, rank, rift, life}
// (rank 0 galaxy, 1 star, 2 planet, 3 moon, 4 comet; rift null when the
// keyfile doesn't say), or null if it isn't one.
export async function keyfileInfo(wasm, keyfile) {
  const X = await bare(wasm);
  const p = cstrIn(X, keyfile), out = X.alloc(32);
  try {
    if (!X.keyfile_ship(p, out)) return null;
    const v = new DataView(X.memory.buffer, out, 32);
    const [lo, hi, rift, life] = [0, 8, 16, 24].map(at => v.getBigUint64(at, true));
    return { ship: takeIn(X, X.ship_render(lo, hi)), rank: X.ship_rank(lo, hi), rift: rift === NONE ? null : rift, life };
  } finally {
    X.free(p);
    X.free(out);
  }
}

// A fake ship's keyfile (urbit -F keys, from its name), for tests that
// must not spend a real identity.
export async function fakeKeyfile(wasm, ship) {
  const X = await bare(wasm);
  const p = cstrIn(X, ship), out = X.alloc(16);
  try {
    if (!X.ship_parse(p, out)) throw new UrbError(`not a ship: ${ship}`);
    const v = new DataView(X.memory.buffer, out, 16);
    return takeIn(X, X.fake_keyfile(v.getBigUint64(0, true), v.getBigUint64(8, true)));
  } finally {
    X.free(p);
    X.free(out);
  }
}

// The @p of a ship number, as [lo, hi] BigInt halves: a moon of parent p
// is (k << 32) | p.
export async function shipName(wasm, lo, hi = 0n) {
  const X = await bare(wasm);
  return takeIn(X, X.ship_render(lo, hi));
}

// The module with no engine and hooks that do nothing, for the helpers above.
async function bare(wasm) {
  const none = () => 0;
  const imports = {
    wasi_snapshot_preview1: wasi(() => X.memory.buffer, () => {}, {}),
    host: { send: none, need_point: none, on_poke: none, on_done: none, on_sage: none, on_peek: () => NONE, store_commit: none },
  };
  const inst = wasm instanceof WebAssembly.Module
    ? await WebAssembly.instantiate(wasm, imports)
    : (await WebAssembly.instantiate(wasm, imports)).instance;
  const X = inst.exports;
  X._initialize();
  return X;
}

function cstrIn(X, s) {
  const b = enc.encode(s + '\0');
  const p = X.alloc(b.length);
  new Uint8Array(X.memory.buffer).set(b, p);
  return p;
}

function takeIn(X, p) {
  const m = new Uint8Array(X.memory.buffer);
  let e = p;
  while (m[e]) e++;
  const s = dec.decode(m.subarray(p, e));
  X.free(p);
  return s;
}

// An error thrown in a hook would trap the module and leave the engine
// unusable: log it instead, and answer as the hook's default would (no
// noun, for on_peek, whose answer is a 64-bit handle).
function guarded(hooks, log) {
  const out = {};
  for (const [name, f] of Object.entries(hooks)) {
    out[name] = (...a) => {
      try {
        return f(...a);
      } catch (e) {
        log(e);
        return name === 'on_peek' ? NONE : 0;
      }
    };
  }
  return out;
}

// The little of WASI mesa.wasm uses: the environment (MESA_TRACE=1 logs
// every packet), stderr and stdout as lines to log, and exit as an error.
function wasi(buffer, log, env) {
  const pending = { 1: '', 2: '' };
  const vars = Object.entries(env).map(([k, v]) => enc.encode(`${k}=${v}\0`));
  return {
    environ_sizes_get(count, size) {
      const v = new DataView(buffer());
      v.setUint32(count, vars.length, true);
      v.setUint32(size, vars.reduce((n, b) => n + b.length, 0), true);
      return 0;
    },
    environ_get(ptrs, buf) {
      const v = new DataView(buffer());
      vars.forEach((b, i) => {
        v.setUint32(ptrs + 4 * i, buf, true);
        new Uint8Array(buffer()).set(b, buf);
        buf += b.length;
      });
      return 0;
    },
    fd_close: () => 0,
    fd_seek: () => 70,   // ESPIPE
    fd_write(fd, iovs, n, written) {
      const v = new DataView(buffer());
      let total = 0;
      for (let i = 0; i < n; i++) {
        const p = v.getUint32(iovs + 8 * i, true), len = v.getUint32(iovs + 8 * i + 4, true);
        pending[fd] = (pending[fd] ?? '') + dec.decode(new Uint8Array(buffer(), p, len));
        total += len;
      }
      const lines = pending[fd].split('\n');
      pending[fd] = lines.pop();
      for (const l of lines) log(l.replace(/\r$/, ''));
      v.setUint32(written, total, true);
      return 0;
    },
    proc_exit(code) {
      throw new Error(`mesa.wasm exited with ${code}`);
    },
  };
}
