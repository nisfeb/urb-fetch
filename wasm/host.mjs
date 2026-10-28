// A Node host for libmesa compiled to wasm (build.sh makes build/mesa.wasm).
// It reads one page published with lattice, as a freshly mined comet, the
// way ../prototype/urb_fetch.py does:
//
//   1. resolve the url as lattice's +de-urb does (the published forms only)
//   2. read the publisher's /pub/revs over grubbery's cross-ship peek
//      (a %grubbery-load poke, answered by a %grubbery-transfer poke back)
//   3. %keen the page body at /g/x/1/grubbery//1/pub/page/<rel>/<id>
//   4. if the list has no entry or the keen misses, peek the vault file
//
//   node host.mjs urb://~sampel-palnet/some/page
//
// The body goes to stdout. Each run mines a new comet and keeps no state,
// so every run leaves one more comet on the publisher.

import dgram from 'node:dgram';
import dns from 'node:dns/promises';
import { randomBytes } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

const DESK_BASE = '/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app';
const APP_BASE = '/apps/lattice.lattice_app';
const REVS_TTL = 60_000;     // +revs-ttl
const KEEN_TIMEOUT = 10_000; // +mesa-timeout
const PEEK_TIMEOUT = 30_000; // +remote-timeout
const ROLLER = 'https://roller.urbit.org/v1/azimuth';
const STARS = 'https://bootstrap.urbit.org/comet-stars.jam';
const KNOT = /^[a-z0-9~._-]+$/;
const NONE = 2n ** 64n - 1n;
// wasm hands 64-bit integers to JS as signed BigInts: read them unsigned
const u64 = x => BigInt.asUintN(64, x);
const ACK = 0;
const LANE_GALAXY = 1, LANE_IP = 2;

const log = (...a) => console.error(...a);
const enc = new TextEncoder(), dec = new TextDecoder();

class UrbError extends Error {}

// ------------------------------------------------------------- the module

const wasi = new WASI({ version: 'preview1', args: [], env: {} });
const { instance } = await WebAssembly.instantiate(
  await readFile(new URL('build/mesa.wasm', import.meta.url)),
  { ...wasi.getImportObject(), host: { send, need_point, on_poke, on_done, on_sage } });
wasi.initialize(instance);
const X = instance.exports;

// memory.buffer is replaced when memory grows: never keep a view of it
const mem = () => new Uint8Array(X.memory.buffer);

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

// ------------------------------------------------------------- ships

// a ship is [lo, hi], two BigInt halves
function shipOf(text) {
  const out = X.alloc(16);
  try {
    if (!withStr(text, p => X.ship_parse(p, out))) throw new UrbError(`not a ship: ${text}`);
    const v = new DataView(X.memory.buffer, out, 16);
    return [v.getBigUint64(0, true), v.getBigUint64(8, true)];
  } finally {
    X.free(out);
  }
}

const shipText = (lo, hi) => take(X.ship_render(lo, hi));

// ------------------------------------------------------------- nouns

// a borrowed noun handle, valid during the hook that handed it out
class Noun {
  constructor(n) { this.n = n; }
  get isCell() { return X.is_cell(this.n) !== 0; }
  get head() { return new Noun(X.head(this.n)); }
  get tail() { return new Noun(X.tail(this.n)); }
  bytes() {
    const len = X.met3(this.n);
    const p = X.alloc(len || 1);
    try {
      X.atom_bytes(this.n, p, len);
      return mem().slice(p, p + len);
    } finally {
      X.free(p);
    }
  }
  text() { return dec.decode(this.bytes()); }
  int() {
    const b = this.bytes();
    let v = 0n;
    for (let i = b.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(b[i]);
    return v;
  }
}

// a new noun (an owned reference) from a JS value: null (~), a boolean (a
// loobean), a number or BigInt, a string (a cord), a Noun, or an array of
// two or more, right-associated into cells
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
  throw new TypeError(`can't make a noun of ${x}`);
}

const atom = bytes => withBytes(bytes, (p, n) => X.atom(p, n));

// a path noun from "/a/b/c", keeping empty segments as stab does
function path(text) {
  const t = text.replace(/^\//, '');
  return t ? [...t.split('/'), null] : null;
}

function* list(n) {
  for (; n.isCell; n = n.tail) yield n.head;
}

// the nodes of a hoon map (a treap of [n l r])
function entries(n) {
  const out = [], stack = [n];
  while (stack.length) {
    const t = stack.pop();
    if (t.isCell) {
      out.push(t.head);
      stack.push(t.tail.head, t.tail.tail);
    }
  }
  return out;
}

const unit = n => (n.isCell ? n.tail : null);

// a $json noun as JS
function json(n) {
  if (!n.isCell) return null;
  const tag = n.head.text(), p = n.tail;
  if (tag === 'o') return Object.fromEntries(entries(p).map(kv => [kv.head.text(), json(kv.tail)]));
  if (tag === 'a') return [...list(p)].map(json);
  if (tag === 'b') return p.int() === 0n;
  if (tag === 'n') return Number(p.text());
  return p.text();
}

const cord = n => (n.isCell ? null : n.text());

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

// ------------------------------------------------------------- the engine

let timer = null;

// after every engine call: wake the engine at its next deadline
function arm() {
  clearTimeout(timer);
  const d = u64(X.deadline());
  if (d === NONE) return;
  //  setTimeout can't wait longer than 2^31 ms; waking early just re-arms
  const wait = Math.min(Math.max(0, Number(d) - Date.now()), 60_000);
  timer = setTimeout(() => {
    if (Date.now() >= Number(u64(X.deadline()))) call(() => X.wake(BigInt(Date.now())));
    else arm();
  }, wait);
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

const sock = dgram.createSocket('udp4');

const ipText = ip => [24, 16, 8, 0].map(s => (ip >>> s) & 0xff).join('.');
const ipNum = a => a.split('.').reduce((v, o) => ((v << 8) | Number(o)) >>> 0, 0);

sock.on('message', (msg, from) => {
  withBytes(msg, (p, n) => call(() => X.hear(ipNum(from.address), from.port, p, n, BigInt(Date.now()))));
});

// galaxy -> {ip, at}; looked up in the background, meanwhile the datagram
// goes nowhere and the engine resends
const galaxies = new Map(), galAsking = new Set();

function galaxy(g) {
  const hit = galaxies.get(g);
  if ((!hit || Date.now() - hit.at > 300_000) && !galAsking.has(g)) {
    galAsking.add(g);
    const name = shipText(BigInt(g), 0n).slice(1) + '.urbit.org';
    dns.lookup(name, { family: 4 })
      .then(r => galaxies.set(g, { ip: r.address, at: Date.now() }))
      .catch(e => log(`can't find galaxy ${name}: ${e.message}`))
      .finally(() => galAsking.delete(g));
  }
  return hit ? [hit.ip, 13337 + g] : null;
}

// hook: send one datagram. Copy it out first: memory can grow before the
// socket reads it
function send(kind, gal, ip, port, ptr, len) {
  const data = Buffer.from(mem().slice(ptr, ptr + len));
  const addr = kind === LANE_GALAXY ? galaxy(gal)
    : kind === LANE_IP && ip && port ? [ipText(ip >>> 0), port]
    : null;
  if (addr) sock.send(data, addr[1], addr[0]);
}

// hook: find a ship's keys on azimuth; answer once the roller has
const asking = new Set();

function need_point(lo, hi) {
  [lo, hi] = [u64(lo), u64(hi)];
  const k = `${lo}:${hi}`;
  if (asking.has(k)) return;
  asking.add(k);
  const body = take(X.point_request(lo, hi));
  fetch(ROLLER, { method: 'POST', headers: { 'content-type': 'application/json' }, body,
                  signal: AbortSignal.timeout(30_000) })
    .then(r => r.text())
    .catch(e => (log(`azimuth lookup of ${shipText(lo, hi)} failed: ${e.message}`), null))
    .then(js => {
      asking.delete(k);
      call(() => (js === null ? X.point(lo, hi, 0, 0)
                              : withBytes(enc.encode(js), (p, n) => X.point(lo, hi, p, n))));
    });
}

// ------------------------------------------------------------- the reader

const peeks = new Map();   // wire -> state of one peek
const keens = new Map();   // scry path -> its resolve
const flows = new Map();   // ship -> bone of our flow to its %grubbery
const bases = new Map();   // ship -> where it runs lattice
const revs = new Map();    // ship -> {at, list}
//  wires never repeat across runs: a publisher can deliver a reply meant
//  for an earlier run late, and it must not match ours
const nonce = randomBytes(4).toString('hex');
let wires = 0;

function poke(her, bone, agent, mark, value) {
  const n = noun(value);
  withStr(agent, a => withStr(mark, m => check(X.poke(her[0], her[1], bone, a, m, n), 'poke')));
}

// grubbery's cross-ship peek of one file: [dest-dir, name]
function peek(her, folder, name, conv) {
  const wire = `/urb-fetch/${nonce}/${++wires}`;
  return new Promise(resolve => {
    const st = { conv, snap: null, done: false };
    st.finish = v => {
      if (st.done) return;
      st.done = true;
      clearTimeout(st.timer);
      peeks.delete(wire);
      if (st.snap) X.unref(st.snap.n);
      resolve(v);   // null: refused, nothing there, or no answer
    };
    st.timer = setTimeout(() => st.finish(null), PEEK_TIMEOUT);
    peeks.set(wire, st);
    txn(() => {
      const k = `${her[0]}:${her[1]}`;
      if (!flows.has(k)) {
        const bone = u64(X.flow_open(her[0], her[1]));
        if (bone === NONE) throw new Error('flow_open failed');
        flows.set(k, bone);
      }
      st.bone = flows.get(k);
      poke(her, st.bone, 'grubbery', 'grubbery-load',
           [[path(wire), [true, path(folder), name]], ['peek', null, true]]);
    });
  });
}

// hook: a poke to us; only %grubbery-transfer answers matter
function on_poke(lo, hi, bone, agentPtr, markN, nounN) {
  [lo, hi, bone] = [u64(lo), u64(hi), u64(bone)];
  try {
    if (cstr(agentPtr) !== 'grubbery' || new Noun(markN).text() !== 'grubbery-transfer') return ACK;
    const n = new Noun(nounN);
    const wire = '/' + [...list(n.head)].map(s => s.text()).join('/');
    const tag = n.tail.head.text(), r = n.tail.tail;
    if (wire === '/want') {
      //  +process-want answers on the fixed wire /want, not ours, so match
      //  the content to the snap that asked for it
      const wants = [...peeks.values()].filter(st => st.snap && !st.done);
      for (const st of wants) {
        const v = tag === 'data' ? fileOf(st.snap, r, st.conv) : null;
        if (v !== null) st.finish(v);
      }
      if (tag === 'miss' && wants.length) wants[0].finish(null);   // the pin expired
      return ACK;
    }
    const st = peeks.get(wire);
    if (!st) return ACK;
    if (tag === 'snap') {
      const snap = unit(r.tail.tail.head), data = unit(r.tail.tail.tail);
      if (snap && !data) {
        //  too big to inline: ask for the content, pinned for 5 minutes
        st.snap = new Noun(X.ref(snap.n));
        poke([lo, hi], st.bone, 'grubbery', 'grubbery-transfer', [n.head, 'want', r.head, r.tail.head.int()]);
        return ACK;
      }
      st.finish(snap ? fileOf(snap, data, st.conv) : null);
      return ACK;
    }
    st.finish(null);
  } catch (e) {
    log(e);
  }
  return ACK;
}

// the lines of an error noun [tag tang] (a nack's naxplanation)
function tang(err) {
  const tape = t => dec.decode(Uint8Array.from([...list(t)].map(c => (c.isCell ? 63 : Number(c.int() & 0xffn)))));
  const tank = (t, depth = 0) => {
    if (depth > 32 || !t.isCell || t.head.isCell) return '';
    const tag = t.head.text();
    if (tag === 'leaf') return tape(t.tail);
    if (tag !== 'rose' && tag !== 'palm') return '';
    const pre = t.tail.head;
    const [sep, opn, cls] = tag === 'rose'
      ? [tape(pre.head), tape(pre.tail.head), tape(pre.tail.tail)]
      : [tape(pre.head), tape(pre.tail.head) + tape(pre.tail.tail.head), tape(pre.tail.tail.tail)];
    return opn + [...list(t.tail.tail)].map(k => tank(k, depth + 1)).join(sep) + cls;
  };
  if (!err.isCell) return '';
  return [err.head.isCell ? '' : err.head.text(), ...[...list(err.tail)].map(t => tank(t))].join('\n  ');
}

// hook: our poke was acked, or nacked
function on_done(lo, hi, bone, error) {
  [bone, error] = [u64(bone), u64(error)];
  if (error === NONE) return;
  log(`nack from ${shipText(lo, hi)}: ${tang(new Noun(error))}`);
  for (const st of [...peeks.values()]) if (st.bone === bone) st.finish(null);
}

function keen(her, spath) {
  return new Promise(resolve => {
    const t = setTimeout(() => {
      //  a miss never answers: stop asking, as +keen-page-raw %yawns
      keens.delete(spath);
      txn(() => withStr(spath, p => X.yawn(her[0], her[1], p)));
      resolve(null);
    }, KEEN_TIMEOUT);
    keens.set(spath, v => {
      clearTimeout(t);
      keens.delete(spath);
      resolve(v);
    });
    txn(() => check(withStr(spath, p => X.keen(her[0], her[1], p)), 'keen'));
  });
}

// hook: the answer to a keen
function on_sage(lo, hi, pathPtr, gage) {
  const done = keens.get(cstr(pathPtr));
  if (!done) return;
  const g = u64(gage) === NONE ? null : new Noun(gage);
  done(g && { mark: g.head.text(), body: cord(g.tail) });
}

// +peer-base: where the peer runs lattice, asked, not guessed
async function base(her, who) {
  if (!bases.has(who)) {
    for (const b of [DESK_BASE, APP_BASE]) {
      const got = await peek(her, b + '/pub', 'revs', json);
      if (got !== null) {
        bases.set(who, b);
        revs.set(who, { at: Date.now(), list: got });
        break;
      }
    }
    if (!bases.has(who)) throw new UrbError(`${who} answered no lattice peek (offline, or no lattice)`);
  }
  return bases.get(who);
}

// +peer-revs-cached
async function revList(her, who) {
  const b = await base(her, who);
  let r = revs.get(who);
  if (!r || Date.now() - r.at > REVS_TTL) {
    r = { at: Date.now(), list: (await peek(her, b + '/pub', 'revs', json)) ?? {} };
    revs.set(who, r);
  }
  return r.list;
}

// +read-page-body-rev: keen the listed id, else peek the vault file
async function page(her, who, rel) {
  const key = '/' + rel.join('/');
  const ent = (await revList(her, who))[key];
  if (ent) {
    const got = await keen(her, `/g/x/1/grubbery//1/pub/page${key}/${ent.id}`);
    if (got && got.mark === 'gmi' && got.body !== null) return [got.body, 'keen'];
    revs.delete(who);   // a miss means the list is stale
  }
  const body = await peek(her, (await base(her, who)) + '/pub/vault' + key, 'gmi', cord);
  return body !== null ? [body, 'peek'] : [null, null];
}

// [ship, rel] for a published page's url, as lattice's +de-urb resolves it
function deUrb(url) {
  if (!url.startsWith('urb://')) throw new UrbError('not an urb:// url');
  const rest = url.slice(6);
  const slash = rest.indexOf('/');
  const who = slash < 0 ? rest : rest.slice(0, slash);
  let tail = slash < 0 ? '' : rest.slice(slash + 1);
  if (tail.endsWith('/')) tail = tail.slice(0, -1);   // urb://~s/site/ = urb://~s/site
  shipOf(who);
  if (!tail) return [who, ['index']];
  const segs = tail.split('/');
  if (!segs.every(s => KNOT.test(s))) throw new UrbError(`bad path in ${url}`);
  if (segs[0].length > 1) return [who, segs];          // frozen legacy form
  if (segs[0] === 'n') return [who, segs.slice(1)];
  if ('pkt'.includes(segs[0])) throw new UrbError(`/${segs[0]}/ addresses read the publisher's tree: not supported`);
  throw new UrbError(`/${segs[0]}/ is not a mount`);
}

// +read-page-or-index for one url: [key, body, how]
async function read(url) {
  const [who, rel] = deUrb(url);
  const her = shipOf(who);
  const list = await revList(her, who);
  const hit = p => ('/' + p.join('/')) in list;
  const dex = [...rel, 'index'];
  const tries = !rel.length || rel.at(-1) === 'index' ? [rel]
    : !hit(rel) && hit(dex) ? [dex]
    : Object.keys(list).length && !hit(rel) ? [rel]
    : [rel, dex];
  for (const p of tries) {
    const [body, how] = await page(her, who, p);
    if (body !== null) return ['/' + p.join('/'), body, how];
  }
  throw new UrbError(`${url}: not published`);
}

// ------------------------------------------------------------- main

const url = process.argv[2];
if (!url) {
  log('usage: node host.mjs urb://~ship/page');
  process.exit(2);
}

let code = 0;
try {
  //  the socket is ready before the engine opens: opening can send
  await new Promise(r => sock.bind(0, r));

  let t = Date.now();
  const res = await fetch(STARS, { headers: { 'user-agent': 'urb-fetch' } });   // 403 without one
  if (!res.ok) throw new Error(`${STARS}: ${res.status}`);
  const jam = new Uint8Array(await res.arrayBuffer());
  log('mining a comet...');
  const keyfile = withBytes(jam, (j, n) => withBytes(randomBytes(64), e => take(X.mine_comet(j, n, e))));
  if (!keyfile) throw new Error('mining failed');
  call(() => check(withStr(keyfile, p => X.open(p, BigInt(Date.now()))), 'open'));
  log(`as ${shipText(X.our_lo(), X.our_hi())}, mined in ${((Date.now() - t) / 1000).toFixed(1)}s`);

  t = Date.now();
  const [key, body, how] = await read(url);
  process.stdout.write(body);
  log(`ok ${url} -> ${key}, ${enc.encode(body).length} bytes, by ${how}, ${((Date.now() - t) / 1000).toFixed(1)}s`);
} catch (e) {
  log(e instanceof UrbError ? `FAIL ${e.message}` : e);
  code = 1;
} finally {
  clearTimeout(timer);
  sock.close();
}
process.exit(code);
