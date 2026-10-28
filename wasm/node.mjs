// Node's side of mesa.mjs, shared by the command-line hosts (host.mjs,
// furum-cli.mjs, pub-cli.mjs): a state folder, UDP, and the options that
// pick who runs.
//
//   --state DIR        where the identity and its state live (default
//                      ~/.local/state/urb-fetch-wasm)
//   --relay URL        send datagrams through `urb-fetch -relay`, not UDP
//   --moon KEYFILE     run as this moon (the key |moon printed); add
//   --new-moon         on its first run only, if it never ran anywhere
//   --fake ~SHIP       run as that ship on a fake network (urbit -F) on this
//                      machine, keys from its name; its state stays until
//                      the fake ships it spoke to are reset, then delete it
//   --trace            log every datagram and the engine's view of it

import dgram from 'node:dgram';
import dns from 'node:dns/promises';
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fakeKeyfile, keyfileInfo, open, readRows, relayTransport, writeRows } from './mesa.mjs';

export const log = (...a) => console.error(new Date().toISOString().slice(14, 23), ...a);
const ipText = ip => [24, 16, 8, 0].map(s => (ip >>> s) & 0xff).join('.');
const ipNum = a => a.split('.').reduce((v, o) => ((v << 8) | Number(o)) >>> 0, 0);

// for parseArgs
export const options = {
  state: { type: 'string' }, relay: { type: 'string' }, trace: { type: 'boolean' },
  moon: { type: 'string' }, 'new-moon': { type: 'boolean' }, fake: { type: 'string' },
};

// the identity's keyfile and the engine's rows, in a folder: the comet's
// keyfile is kept there, a moon's or a fake ship's comes from outside.
// Every commit rewrites the rows file whole.
// ponytail: no lock, so two runs on one folder would run the identity twice.
export function fileStore(dir, given = null) {
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
      if (given) keyfile = given.keyfile;
      else try { keyfile = readFileSync(keyPath, 'utf8').trim(); } catch {}
      try { saved = new Uint8Array(readFileSync(rowsPath)); } catch {}
      if (saved) for (const r of readRows(saved)) rows.set(hex(r.key), r);
      return { keyfile, rows: saved, fresh: Boolean(given && given.fresh) };
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

// UDP from one port per identity, kept across runs: publishers answer the
// address they last saw, so a new port each run leaves them resending to a
// dead one, with newer answers queued behind. On a fake network every ship
// is on this machine, galaxy g at port 31337 + g.
export function udpTransport(dir, fake = false) {
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
    if (fake) return ['127.0.0.1', 31337 + g];
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
        sock.bind(port, fake ? '127.0.0.1' : undefined, resolve);
      });
    },
    send(lane, bytes) {
      const addr = lane.name ? galaxy(lane.galaxy, lane.name) : [fake ? '127.0.0.1' : ipText(lane.ip), lane.port];
      if (addr) sock.send(bytes, addr[1], addr[0]);
    },
    close() {
      sock.close();
    },
  };
}

// every datagram, logged
export function traced(t) {
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

// the engine, as the options say
export async function connect(opt) {
  const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
  let dir = opt.state || join(process.env.XDG_STATE_HOME || join(homedir(), '.local/state'), 'urb-fetch-wasm');
  let given = null;
  if (opt.fake) {
    if (opt.relay) throw new Error('a fake network is on this machine: no relay');
    given = { keyfile: await fakeKeyfile(wasm, opt.fake), fresh: true };
    dir = join(dir, `fake-${opt.fake.replace(/^~/, '')}`);
  } else if (opt.moon) {
    const keyfile = readFileSync(opt.moon, 'utf8').trim();
    const info = await keyfileInfo(wasm, keyfile);
    if (!info || info.rank !== 3) throw new Error(`${opt.moon} holds no moon's key`);
    log(`moon ${info.ship}, life ${info.life}, rift ${info.rift ?? 'not in the keyfile'}`);
    given = { keyfile, fresh: Boolean(opt['new-moon']) };
    dir = join(dir, info.ship.slice(1));
  }
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const t = Date.now();
  const transport = opt.relay ? relayTransport(opt.relay, log) : udpTransport(dir, Boolean(opt.fake));
  const urb = await open({
    wasm,
    transport: opt.trace ? traced(transport) : transport,
    store: fileStore(dir, given),
    log,
    env: opt.trace ? { MESA_TRACE: '1' } : {},
    fake: Boolean(opt.fake),
  });
  log(`as ${urb.our}, ready in ${((Date.now() - t) / 1000).toFixed(1)}s${opt.relay ? ` through ${opt.relay}` : opt.fake ? ' on the fake network' : ''}`);
  return Object.assign(urb, { dir });
}
