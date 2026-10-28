// Reads one page published with lattice from Node, as a comet or a moon
// run by libmesa compiled to wasm (mesa.mjs, build/mesa.wasm):
//
//   node host.mjs [--state DIR] [--relay ws://localhost:8476]
//                 [--moon KEYFILE [--new-moon]] urb://~ship/page
//
// The body goes to stdout. The comet is mined on the first run and kept in
// the state folder (default ~/.local/state/urb-fetch-wasm) with the
// engine's state, so later runs are the same comet. A moon's state is kept
// in a folder of its own there. Pass --new-moon only on a moon's first run,
// and only if it never ran anywhere else. Datagrams go over UDP from a port
// kept beside the state, or through `urb-fetch -relay` with --relay.

import dgram from 'node:dgram';
import dns from 'node:dns/promises';
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { keyfileInfo, open, readRows, relayTransport, writeRows, UrbError } from './mesa.mjs';

const log = (...a) => console.error(new Date().toISOString().slice(14, 23), ...a);
const ipText = ip => [24, 16, 8, 0].map(s => (ip >>> s) & 0xff).join('.');
const ipNum = a => a.split('.').reduce((v, o) => ((v << 8) | Number(o)) >>> 0, 0);

// the identity's keyfile and the engine's rows, in a folder: the comet's
// keyfile is kept there, a moon's comes from its own file. Every commit
// rewrites the rows file whole.
// ponytail: no lock, so two runs on one folder would run the identity twice.
function fileStore(dir, moon = null) {
  const keyPath = join(dir, 'comet.key'), rowsPath = join(dir, 'rows');
  const rows = new Map();   // key as hex -> {key, val}
  const hex = b => Buffer.from(b).toString('hex');
  const save = (path, data) => {
    writeFileSync(path + '.new', data, { mode: 0o600 });
    renameSync(path + '.new', path);
  };
  return {
    load() {
      let keyfile = null, saved = null;
      if (moon) keyfile = moon.keyfile;
      else try { keyfile = readFileSync(keyPath, 'utf8').trim(); } catch {}
      try { saved = new Uint8Array(readFileSync(rowsPath)); } catch {}
      if (saved) for (const r of readRows(saved)) rows.set(hex(r.key), r);
      return { keyfile, rows: saved, fresh: Boolean(moon && moon.fresh) };
    },
    saveKey(keyfile) {
      save(keyPath, keyfile + '\n');
      rows.clear();
      save(rowsPath, new Uint8Array());
    },
    commit(batch) {
      for (const r of readRows(batch)) {
        if (r.val) rows.set(hex(r.key), r);
        else rows.delete(hex(r.key));
      }
      save(rowsPath, writeRows([...rows.values()]));
    },
  };
}

// UDP from one port per comet, kept across runs: publishers answer the
// address they last saw, so a new port each run leaves them resending to a
// dead one, with newer answers queued behind.
function udpTransport(dir) {
  const portPath = join(dir, 'port');
  let port;
  try { port = Number(readFileSync(portPath, 'utf8')); } catch {}
  if (!port) {
    port = 20000 + Math.floor(Math.random() * 40000);
    writeFileSync(portPath, String(port));
  }
  const sock = dgram.createSocket('udp4');

  // galaxy -> {ip, at}, looked up in the background; meanwhile its
  // datagrams go nowhere and the engine resends them
  const galaxies = new Map(), asking = new Set();
  function galaxy(g, name) {
    const hit = galaxies.get(g);
    if ((!hit || Date.now() - hit.at > 300_000) && !asking.has(g)) {
      asking.add(g);
      dns.lookup(`${name}.urbit.org`, { family: 4 })
        .then(r => galaxies.set(g, { ip: r.address, at: Date.now() }))
        .catch(e => log(`can't find galaxy ${name}: ${e.message}`))
        .finally(() => asking.delete(g));
    }
    return hit ? [hit.ip, 13337 + g] : null;
  }

  return {
    start(ship, onPacket) {
      sock.on('message', (msg, from) => onPacket(ipNum(from.address), from.port, msg));
      return new Promise((resolve, reject) => {
        sock.once('error', reject);
        sock.bind(port, resolve);
      });
    },
    send(lane, bytes) {
      const addr = lane.name ? galaxy(lane.galaxy, lane.name) : [ipText(lane.ip), lane.port];
      if (addr) sock.send(bytes, addr[1], addr[0]);
    },
    close() {
      sock.close();
    },
  };
}

// every datagram, logged
function traced(t) {
  const lane = l => (l.name ? `~${l.name}` : `${ipText(l.ip)}:${l.port}`);
  return {
    start: (ship, onPacket) => t.start(ship, (ip, port, bytes) => {
      log(`hear ${bytes.length} from ${ipText(ip)}:${port}`);
      onPacket(ip, port, bytes);
    }),
    send(l, bytes) {
      log(`send ${bytes.length} to ${lane(l)}`);
      t.send(l, bytes);
    },
    close: () => t.close(),
  };
}

const { values: opt, positionals: [url] } = parseArgs({
  allowPositionals: true,
  options: {
    state: { type: 'string' }, relay: { type: 'string' }, trace: { type: 'boolean' }, tries: { type: 'string', default: '1' },
    moon: { type: 'string' }, 'new-moon': { type: 'boolean' },
  },
});
if (!url) {
  log('usage: node host.mjs [--state DIR] [--relay ws://host:port] [--moon KEYFILE [--new-moon]] [--tries N] [--trace] urb://~ship/page');
  process.exit(2);
}
const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
let dir = opt.state || join(process.env.XDG_STATE_HOME || join(homedir(), '.local/state'), 'urb-fetch-wasm');
let moon = null;
if (opt.moon) {
  const keyfile = readFileSync(opt.moon, 'utf8').trim();
  const info = await keyfileInfo(wasm, keyfile);
  if (!info || info.rank !== 3) {
    log(`${opt.moon} holds no moon's key`);
    process.exit(2);
  }
  log(`moon ${info.ship}, life ${info.life}, rift ${info.rift ?? 'not in the keyfile'}`);
  moon = { keyfile, fresh: Boolean(opt['new-moon']) };
  dir = join(dir, info.ship.slice(1));
}
mkdirSync(dir, { recursive: true, mode: 0o700 });

let code = 0, urb = null;
try {
  let t = Date.now();
  const transport = opt.relay ? relayTransport(opt.relay, log) : udpTransport(dir);
  urb = await open({
    wasm,
    transport: opt.trace ? traced(transport) : transport,
    store: fileStore(dir, moon),
    log,
    env: opt.trace ? { MESA_TRACE: '1' } : {},
  });
  log(`as ${urb.our}, ready in ${((Date.now() - t) / 1000).toFixed(1)}s${opt.relay ? ` through ${opt.relay}` : ''}`);
  t = Date.now();
  //  a ship can take a minute or two to answer a comet that moved or
  //  restarted: it is still resending answers meant for earlier runs, in
  //  order, and the engine keeps running between tries
  let got;
  for (let i = 1; !got; i++) {
    try {
      got = await urb.read(url);
    } catch (e) {
      if (!(e instanceof UrbError) || i >= Number(opt.tries)) throw e;
      log(`try ${i}: ${e.message}`);
    }
  }
  const { key, body, how } = got;
  process.stdout.write(body);
  log(`ok ${url} -> ${key}, ${new TextEncoder().encode(body).length} bytes, by ${how}, ${((Date.now() - t) / 1000).toFixed(1)}s`);
} catch (e) {
  log(e instanceof UrbError ? `FAIL ${e.message}` : e);
  code = 1;
} finally {
  await urb?.close();
}
process.exit(code);
