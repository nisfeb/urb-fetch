#!/usr/bin/env node
// Release gate 3, the state migration (docs/rust-core.md), on the fake
// fleet: saved state written by today's code is opened by the Rust code
// and carries on, and the other way, with no breach and no stall.
//
//   FLEET_WHO=<claim> node core/ci/migrate.mjs [--out DIR]
//
// 1. The extension's saved state, kept as the extension keeps it in
//    IndexedDB (rows put and deleted by key and handed back in the keys'
//    binary order, the keyfile, and meta.pub), for a fake comet, as
//    `npm run urb:fleet` runs one: today's module (wasm/mesa.mjs on
//    glue.c's mesa.wasm) from empty, then the Rust one (shim/ on
//    shim/build/mesa.wasm) on what it saved, then today's again on what
//    the Rust one saved. Each session reads ~hus's pages and ~rup's
//    furum board, and publishes a new revision of a page that ~hus's
//    lattice then reads from us, so pages and keepers carry across. The
//    same again starting from the Rust module.
//    A fake moon can't run this: mesa.mjs (and the core, which ports it)
//    breaches a fake ship at every open, and a ship that heard the
//    identity before ignores it after (M1's findings), whichever module
//    opens it. shim/rows.mjs covers a moon's rows offline, byte for byte.
// 2. The Go gateway's state folder for a fake moon of ~hus: Go from
//    empty, then the Rust gateway on that folder, then Go again on the
//    folder the Rust gateway left (the rollback). Neither gateway
//    breaches a fake ship that has state, so ~hus remembers the moon's
//    flows: an empty folder as the same moon gets no answer, and so does
//    Go on its own folder from before the Rust run, if the Rust run moved
//    the flows on.
//
// It needs the furum set claimed by FLEET_WHO, its m1c-traces snapshot,
// wasm/build/mesa.wasm, shim/build/mesa.wasm (shim/build.sh), urb-fetch
// (GO_GATEWAY) and urb-gateway (URB_GATEWAY). It leaves the set on the
// snapshot. Exit 0 only if every check passes.

import { spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { settle } from './settle.mjs';
import { udpTransport } from '../../wasm/node.mjs';
import * as OLD from '../../wasm/mesa.mjs';
import * as NEW from '../../shim/mesa.mjs';
import { furum as oldFurum } from '../../wasm/furum.mjs';
import { furum as newFurum } from '../../shim/furum.mjs';
import { publish as oldPublish } from '../../wasm/pub.mjs';
import { publish as newPublish } from '../../shim/pub.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const FLEET = process.env.FLEET ?? `${process.env.HOME}/software/personal/hoon-development/fleet/fleet`;
const COOKIE = process.env.COOKIE ?? `${process.env.HOME}/software/personal/furum/scripts/hoon-test-kit/ship-cookie.sh`;
const GO = process.env.GO_GATEWAY ?? join(root, 'urb-fetch');
const RUST = process.env.URB_GATEWAY ?? join(root, 'core/target/release/urb-gateway');
const WHO = process.env.FLEET_WHO;
const SNAP = 'm1c-traces';
const HUS = 'http://localhost:8134';
// trace 1's comet-stars.jam, so mining a comet fetches nothing live
const STARS = '8101182800030570e0001838800a';

const argv = process.argv.slice(2);
const out = argv[0] === '--out' ? argv[1] : mkdtempSync(join(tmpdir(), 'migrate-'));
mkdirSync(out, { recursive: true });
const skip = why => { console.error(`migrate: SKIPPED, not passed: ${why}`); process.exit(2); };
for (const f of [GO, RUST, join(root, 'wasm/build/mesa.wasm'), join(root, 'shim/build/mesa.wasm')]) if (!existsSync(f)) skip(`no ${f}`);
if (!WHO || !spawnSync(FLEET, ['ls'], { encoding: 'utf8' }).stdout.split('\n').some(l => l.startsWith('furum ') && l.includes(`(${WHO})`))) {
  skip('the furum set is not claimed by FLEET_WHO');
}

const sleep = ms => new Promise(r => setTimeout(r, ms));
const hex = b => Buffer.from(b).toString('hex');
let failed = 0;
const lines = [];
function check(what, ok, detail = '') {
  const l = `${ok ? 'ok  ' : 'FAIL'} ${what}${detail ? `: ${String(detail).slice(0, 300)}` : ''}`;
  console.log(l);
  lines.push(l);
  if (!ok) failed++;
}

let cookie = null;
function reset() {
  const r = spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`fleet reset: ${r.stderr}`);
  const jar = join(out, 'hus.jar');
  let c;
  for (let i = 1; ; i++) {
    c = spawnSync(COOKIE, [`${process.env.HOME}/fleet/piers/hus`, HUS, jar], { encoding: 'utf8' });
    if (c.status === 0 || i >= 5) break;
    spawnSync('sleep', ['5']);
  }
  if (c.status !== 0) throw new Error(`ship-cookie: ${c.stderr}`);
  const f = readFileSync(jar, 'utf8').split('\n').find(l => l.includes('urbauth-')).split('\t');
  cookie = `${f[5]}=${f[6]}`;
  settle();
}
async function husFetch(url) {
  try {
    const r = await fetch(`${HUS}/apps/lattice/fetch?url=${encodeURIComponent(url)}`,
      { headers: { cookie }, signal: AbortSignal.timeout(90_000) });
    return `${r.status} ${await r.text()}`;
  } catch (e) {
    return `failed: ${e.message}`;
  }
}

// ------------------------------------------------- the extension's state

const MODS = {
  today: { m: OLD, wasm: readFileSync(join(root, 'wasm/build/mesa.wasm')), furum: oldFurum, publish: oldPublish },
  rust: { m: NEW, wasm: readFileSync(join(root, 'shim/build/mesa.wasm')), furum: newFurum, publish: newPublish },
};

// what the extension keeps in IndexedDB for one identity
const idb = () => ({ keyfile: null, rows: new Map(), pub: null, dir: mkdtempSync(join(out, 'idb-')) });
const clone = s => ({ ...s, rows: new Map(s.rows), pub: s.pub && JSON.parse(JSON.stringify(s.pub)) });

async function timed(p) {
  const t = Date.now();
  try {
    return { ok: true, v: await p, s: (Date.now() - t) / 1000 };
  } catch (e) {
    return { ok: false, v: e.message, s: (Date.now() - t) / 1000 };
  }
}

// one session of one module on st: reads, furum, a page published and
// read back by ~hus's lattice; st is updated as IndexedDB would be
async function session(label, which, st, body) {
  const { m, wasm, furum, publish } = MODS[which];
  const loaded = st.rows.size;
  const store = {
    load() {
      const keys = [...st.rows.keys()].sort((a, b) => Buffer.compare(Buffer.from(a, 'hex'), Buffer.from(b, 'hex')));
      const rows = keys.length ? m.writeRows(keys.map(k => ({ key: Buffer.from(k, 'hex'), val: st.rows.get(k) }))) : null;
      return { keyfile: st.keyfile, rows, fresh: !st.keyfile };
    },
    saveKey(k) { st.keyfile = k; st.rows.clear(); },
    commit(batch) {
      for (const { key, val } of m.readRows(batch)) {
        if (val) st.rows.set(hex(key), val);
        else st.rows.delete(hex(key));
      }
    },
  };
  const opened = await timed(m.open({
    wasm, transport: udpTransport(st.dir, true), store, fake: true, log: () => {},
    fetch: async () => new Response(Buffer.from(STARS, 'hex')),
  }));
  check(`${label}: ${which} opens on ${loaded} saved rows`, opened.ok, opened.ok ? `as ${opened.v.our}` : opened.v);
  if (!opened.ok) return null;
  const urb = opened.v;
  if (st.our) check(`${label}: the same identity`, urb.our === st.our, `${st.our} then ${urb.our}`);
  st.our = urb.our;
  const r1 = await timed(urb.read('urb://~hus/fleet-check'));
  check(`${label}: reads ~hus/fleet-check, no stall (${r1.s}s)`, r1.ok && /fleet/.test(r1.v.body) && r1.s < 20, r1.ok ? '' : r1.v);
  const r2 = await timed(urb.read('urb://~hus/big'));
  check(`${label}: reads ~hus/big, ${r2.ok ? r2.v.body.length : 0} chars (${r2.s}s)`, r2.ok && r2.v.body.length > 15_000 && r2.s < 20, r2.ok ? '' : r2.v);
  const f = await timed(furum(urb, '~rup').posts('m1c'));
  check(`${label}: reads ~rup's furum board m1c (${f.s}s)`, f.ok && Array.isArray(f.v) && f.v.length > 0 && f.s < 20, f.ok ? '' : f.v);
  // publishing: meta.pub as saved, a new revision, read by ~hus. The
  // first session has ~hus's lattice follow the page (/sub), so ~hus
  // keeps its vault file; later sessions must carry the keep and wave it
  const before = st.pub && JSON.parse(JSON.stringify(st.pub));
  const want = (before?.pages?.['/migrate']?.rev ?? 0) + 1;
  const pub = publish(urb, { ...(st.pub ?? {}), save: all => { st.pub = JSON.parse(JSON.stringify(all)); } });
  const husKeeps = x => (x?.keepers ?? []).some(k => k.ship === '~hus');
  if (before) {
    const p = pub.pages()['/migrate'];
    check(`${label}: the saved page is carried (rev ${p?.rev})`,
      p && p.body === before.pages['/migrate'].body && p.rev === before.pages['/migrate'].rev);
    check(`${label}: the saved keep is carried (${before.keepers.length} keepers)`, husKeeps(before), JSON.stringify(before.keepers));
  }
  const rec = pub.put('migrate', body, 'md');
  check(`${label}: publishes /migrate rev ${rec?.rev}`, rec && rec.rev === want);
  const url = `urb://${urb.our}/migrate`;
  const seen = await husFetch(url);
  let got = null;
  try { got = JSON.parse(seen.slice(4)).body; } catch {}
  check(`${label}: ~hus's lattice reads the new revision`, seen.startsWith('200') && got === body, seen.slice(0, 200));
  if (!before) {
    const r = await fetch(`${HUS}/apps/lattice/sub?url=${encodeURIComponent(url)}`, { method: 'POST', headers: { cookie }, signal: AbortSignal.timeout(60_000) });
    for (let i = 0; i < 60 && !husKeeps(st.pub); i++) await sleep(500);
    check(`${label}: ~hus follows the page (/sub ${r.status}) and its keep is saved in meta.pub`, husKeeps(st.pub), JSON.stringify(st.pub?.keepers));
  } else {
    await sleep(3000);
    check(`${label}: ~hus still keeps the page after the edit`, husKeeps(st.pub), JSON.stringify(st.pub?.keepers));
  }
  const c = await timed(urb.close());
  check(`${label}: closes`, c.ok, c.ok ? '' : c.v);
  writeFileSync(join(out, `${label.replace(/\W+/g, '-')}.state.json`), JSON.stringify({
    our: st.our, keyfile: st.keyfile, rows: Object.fromEntries([...st.rows].map(([k, v]) => [k, hex(v)])), pub: st.pub,
  }, null, 1));
  return urb.our;
}

reset();
for (const [first, second] of [['today', 'rust'], ['rust', 'today']]) {
  const st = idb();
  const tag = `ext ${first}->${second}->${first}`;
  await session(`${tag} 1`, first, st, `# Migrate\n\nrevision 1, written by ${first}`);
  const before = clone(st);
  await session(`${tag} 2`, second, st, `# Migrate\n\nrevision 2, written by ${second}`);
  check(`${tag}: ${second} saved over ${first}'s rows`, [...st.rows].some(([k, v]) => hex(before.rows.get(k) ?? []) !== hex(v)));
  await session(`${tag} 3`, first, st, `# Migrate\n\nrevision 3, written by ${first} again`);
}

// ------------------------------------------------- the gateway's state

const MOON = '~natdul-molryp-dozzod-dozhus';
const base = `fake-${MOON.slice(1)}`;
const AT = '127.0.0.4', PORT = '8476';
async function start(bin, state) {
  const p = spawn(bin, ['-fake', MOON, '-state', state, '-listen', `${AT}:${PORT}`]);
  let log = '';
  p.stdout.on('data', d => { log += d; });
  p.stderr.on('data', d => { log += d; });
  const exited = new Promise(r => p.on('exit', (c, s) => r(c ?? `signal ${s}`)));
  const t = Date.now();
  while (!log.includes('serving http://') && p.exitCode === null && Date.now() - t < 60_000) await sleep(100);
  if (!log.includes('serving http://')) throw new Error(`${bin} did not start: ${log}`);
  return {
    log: () => log,
    stop: () => { p.kill('SIGINT'); return Promise.race([exited, sleep(15_000).then(() => { p.kill('SIGKILL'); return 'hung'; })]); },
  };
}
function get(path) {
  return new Promise(resolve => {
    const t = Date.now();
    let got = '';
    const s = connect(Number(PORT), AT, () => s.write(`GET ${path} HTTP/1.1\r\nHost: hus.urb.localhost:${PORT}\r\nConnection: close\r\n\r\n`));
    s.on('data', d => { got += d; });
    s.on('close', () => resolve({ status: got.split('\r\n')[0], body: got.slice(got.indexOf('\r\n\r\n') + 4), s: (Date.now() - t) / 1000 }));
    s.on('error', e => resolve({ status: `error ${e.message}`, body: '', s: (Date.now() - t) / 1000 }));
  });
}
async function gateway(label, bin, state, expect = true) {
  const port = existsSync(join(state, `${base}.db.port`)) ? readFileSync(join(state, `${base}.db.port`), 'utf8').trim() : null;
  const g = await start(bin, state);
  const pages = [];
  for (const path of ['/fleet-check', '/big', '/marks/md', '/marks/html']) {
    const a = await get(path);
    pages.push([path, a]);
    if (expect) check(`${label}: ${path} ${a.status} (${a.s}s)`, a.status === 'HTTP/1.1 200 OK' && a.s < 20);
  }
  const stopped = await g.stop();
  writeFileSync(join(out, `${label.replace(/\W+/g, '-')}.log`), g.log());
  if (port) check(`${label}: kept UDP port ${port}`, readFileSync(join(state, `${base}.db.port`), 'utf8').trim() === port);
  check(`${label}: stops on Ctrl-C (${stopped})`, stopped === 0 || stopped === 'signal SIGINT' || stopped === 130, stopped);
  return pages;
}

reset();
const s1 = join(out, 'gw-1-go'), s2 = join(out, 'gw-2-rust'), s3 = join(out, 'gw-3-go');
await gateway('gateway 1 Go from empty', GO, s1);
cpSync(s1, s2, { recursive: true });
await gateway('gateway 2 Rust on Go\'s folder', RUST, s2);
cpSync(s2, s3, { recursive: true });
await gateway('gateway 3 Go on the Rust gateway\'s folder (rollback)', GO, s3);
// controls: an empty folder, and Go's folder from before the Rust run
const c1 = await gateway('control: empty folder (Go)', GO, join(out, 'gw-control-empty'), false);
check(`control: an empty folder as ${MOON} gets no page`, c1.every(([, a]) => a.status !== 'HTTP/1.1 200 OK'), c1.map(([p, a]) => `${p} ${a.status}`).join(', '));
const stale = join(out, 'gw-control-stale');
cpSync(s1, stale, { recursive: true });
const c2 = await gateway('control: Go on its folder from before the Rust run', GO, stale, false);
console.log(`info control: stale Go folder: ${c2.map(([p, a]) => `${p} ${a.status} ${a.s}s`).join(', ')}`);
lines.push(`info stale: ${c2.map(([p, a]) => `${p} ${a.status} ${a.s}s`).join(', ')}`);

spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
writeFileSync(join(out, 'summary.txt'), lines.join('\n') + '\n');
console.log(`\nmigrate: ${failed ? `${failed} FAILED` : 'every check passed'} (outputs in ${out})`);
process.exit(failed ? 1 : 0);
