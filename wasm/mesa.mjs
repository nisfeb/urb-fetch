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
//   store      where the comet and the engine's state are kept:
//              {load() -> {keyfile, rows}, saveKey(keyfile), commit(rows)},
//              each may return a promise. rows are glue.c's
//              [klen:4][key][vlen:4][val]; readRows and writeRows convert.
//
//   const urb = await open({ wasm, transport, store })
//   const { key, body, how } = await urb.read('urb://~sampel-palnet/page')

const DESK_BASE = '/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app';
const APP_BASE = '/apps/lattice.lattice_app';
const REVS_TTL = 60_000;     // +revs-ttl
const KEEN_TIMEOUT = 10_000; // +mesa-timeout
const PEEK_TIMEOUT = 30_000; // +remote-timeout
const ROLLER = 'https://roller.urbit.org/v1/azimuth';
const STARS = 'https://bootstrap.urbit.org/comet-stars.jam';
const KNOT = /^[a-z0-9~._-]+$/;
const NONE = 2n ** 64n - 1n;
const ACK = 0;
const LANE_GALAXY = 1, LANE_IP = 2;
const GONE = 0xffffffff;     // a deleted row's vlen

// wasm hands 64-bit integers to JS as signed BigInts: read them unsigned
const u64 = x => BigInt.asUintN(64, x);
const enc = new TextEncoder(), dec = new TextDecoder();

export class UrbError extends Error {}

// ------------------------------------------------------------- addresses

// [ship, rel] for a published page's url, as lattice's +de-urb resolves it.
// /p/, /k/ and /t/ name the publisher's tree, which needs its grants.
export function deUrb(url) {
  if (!url.startsWith('urb://')) throw new UrbError('not an urb:// url');
  const rest = url.slice(6);
  const slash = rest.indexOf('/');
  const who = slash < 0 ? rest : rest.slice(0, slash);
  let tail = slash < 0 ? '' : rest.slice(slash + 1);
  if (tail.endsWith('/')) tail = tail.slice(0, -1);   // urb://~s/site/ = urb://~s/site
  if (!/^~[a-z]+(-[a-z]+)*$/.test(who)) throw new UrbError(`no ship in ${url}`);
  if (!tail) return [who, ['index']];
  const segs = tail.split('/');
  if (!segs.every(s => KNOT.test(s))) throw new UrbError(`bad path in ${url}`);
  if (segs[0].length > 1) return [who, segs];          // frozen legacy form
  if (segs[0] === 'n') return [who, segs.slice(1)];
  if ('pkt'.includes(segs[0])) throw new UrbError(`/${segs[0]}/ addresses read the publisher's tree: not supported`);
  throw new UrbError(`/${segs[0]}/ is not a mount`);
}

// lattice-gmi's +looks-html: the first non-blank character is <
export const looksHtml = body => body.slice(0, 256).trimStart().startsWith('<');

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

export async function open({ wasm, transport, store, log = console.error, env = {} }) {
  let X;
  const mem = () => new Uint8Array(X.memory.buffer);   // replaced when memory grows
  const imports = {
    wasi_snapshot_preview1: wasi(() => X.memory.buffer, log, env),
    host: guarded({ send, need_point, on_poke, on_done, on_sage, store_commit }, log),
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

  // a new noun (an owned reference) from a JS value: null (~), a boolean
  // (a loobean), a number or BigInt, a string (a cord), a Noun, or an
  // array of two or more, right-associated into cells
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

  // ----------------------------------------------------------- the loop

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

  function onPacket(ip, port, bytes) {
    withBytes(bytes, (p, n) => call(() => X.hear(ip, port, p, n, BigInt(Date.now()))));
  }

  // ----------------------------------------------------------- the reader

  const peeks = new Map();   // wire -> state of one peek
  const keens = new Map();   // scry path -> {done, promise}
  const flows = new Map();   // ship -> bone of our flow to its %grubbery
  const bases = new Map();   // ship -> where it runs lattice
  const revs = new Map();    // ship -> {at, list}
  //  wires never repeat across runs: a publisher can deliver a reply meant
  //  for an earlier run late, and it must not match ours
  const nonce = [...crypto.getRandomValues(new Uint8Array(4))].map(b => b.toString(16).padStart(2, '0')).join('');
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
    return ACK;
  }

  // hook: our poke was acked, or nacked
  function on_done(lo, hi, bone, error) {
    [lo, hi, bone, error] = [u64(lo), u64(hi), u64(bone), u64(error)];
    if (error === NONE) return;
    log(`nack from ${shipText(lo, hi)}: ${tang(new Noun(error))}`);
    for (const st of [...peeks.values()]) if (st.bone === bone) st.finish(null);
  }

  // one keen at a time per path; readers asking the same share it
  function keen(her, spath) {
    if (keens.has(spath)) return keens.get(spath).promise;
    const k = {};
    k.promise = new Promise(resolve => {
      const t = setTimeout(() => {
        //  a miss never answers: stop asking, as +keen-page-raw %yawns
        keens.delete(spath);
        txn(() => withStr(spath, p => X.yawn(her[0], her[1], p)));
        resolve(null);
      }, KEEN_TIMEOUT);
      k.done = v => {
        clearTimeout(t);
        keens.delete(spath);
        resolve(v);
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
    const g = u64(gage) === NONE ? null : new Noun(gage);
    k.done(g && { mark: g.head.text(), body: cord(g.tail) });
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

  // +read-page-or-index for one url
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
      if (body !== null) return { key: '/' + p.join('/'), body, how };
    }
    throw new UrbError(`${url}: not published`);
  }

  // ----------------------------------------------------------- start

  let { keyfile, rows } = await store.load();
  if (!keyfile) {
    const res = await fetch(STARS, { headers: { 'user-agent': 'urb-fetch' } });   // 403 without one, outside a browser
    if (!res.ok) throw new Error(`${STARS}: ${res.status}`);
    const jam = new Uint8Array(await res.arrayBuffer());
    keyfile = withBytes(jam, (j, n) => withBytes(crypto.getRandomValues(new Uint8Array(64)), e => take(X.mine_comet(j, n, e))));
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
  call(() => check(withStr(keyfile, p => X.open(p, BigInt(Date.now()))), 'open'));

  return {
    our,
    read,
    // resolves once everything committed is saved
    close() {
      clearTimeout(timer);
      transport.close();
      X.close();
      return saved;
    },
  };
}

// An error thrown in a hook would trap the module and leave the engine
// unusable: log it instead, and answer as the hook's default would.
function guarded(hooks, log) {
  const out = {};
  for (const [name, f] of Object.entries(hooks)) {
    out[name] = (...a) => {
      try {
        return f(...a);
      } catch (e) {
        log(e);
        return 0;
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
