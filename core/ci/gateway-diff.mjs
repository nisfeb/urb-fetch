#!/usr/bin/env node
// The differential run of M8 (docs/rust-core.md): the Go gateway
// (urb-fetch) and the Rust one (urb-gateway) answer the same fixed list
// of requests from the fake fleet, and their answers must be the same:
// status line, headers and body, byte for byte, once the narrow
// normalisation below is applied.
//
//   FLEET_WHO=lattice-loop-m8 node core/ci/gateway-diff.mjs [--out DIR] [NAME...]
//   node core/ci/gateway-diff.mjs --offline     the startup cases only
//
// The run is also the state migration test. The Go gateway runs first,
// as a fake moon of ~hus, on a fresh state folder, and answers the list.
// It is stopped, its folder is copied, and the Rust gateway opens the
// COPY, as the same moon, and answers the list again. On the fake
// network an identity heard before can't start again from empty (the
// control case shows it: a fresh folder as the same moon gets no
// answer), so every page the Rust side reads proves it continued the Go
// gateway's flows from its <ship>.db. It must also bind the same UDP
// port, from <ship>.db.port, which is checked while it runs. The comet
// case (offline, in a network namespace of its own) opens a Go-written
// comet.key and its <comet>.db and port file the same way.
//
// It needs the furum fleet set claimed by FLEET_WHO and its m1c-traces
// snapshot, urb-fetch built (make urb-fetch; GO_GATEWAY), urb-gateway
// built (cargo build --release -p urb-gateway; URB_GATEWAY), and go
// for the comet case. Without them it says what is missing and exits 2,
// a skip, never a pass. It leaves the set on the snapshot. The startup
// cases need no ship, and run first (and alone with --offline, in CI).

import { spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const FLEET = process.env.FLEET ?? `${process.env.HOME}/software/personal/hoon-development/fleet/fleet`;
const GO = process.env.GO_GATEWAY ?? join(root, 'urb-fetch');
const RUST = process.env.URB_GATEWAY ?? join(root, 'core/target/release/urb-gateway');
const WHO = process.env.FLEET_WHO;
const SNAP = 'm1c-traces';
// both listen on port 8475, each on a loopback address of its own, so
// the links each renders name the same port
const PORT = '8475';
const AT = { go: '127.0.0.2', rust: '127.0.0.3' };
const MOON = '~natdul-molbud-dozzod-dozhus';

const argv = process.argv.slice(2);
let out = null, offlineOnly = false;
if (argv[0] === '--offline') { offlineOnly = true; argv.shift(); }
if (argv[0] === '--out') { out = argv[1]; argv.splice(0, 2); }
out ??= mkdtempSync(join(tmpdir(), 'gateway-diff-'));
mkdirSync(out, { recursive: true });
const only = new Set(argv);
const want = name => !only.size || only.has(name);

const skip = why => { console.error(`gateway-diff: SKIPPED, not passed: ${why}`); process.exit(2); };
if (!existsSync(GO)) skip(`no Go gateway at ${GO} (make urb-fetch)`);
if (!existsSync(RUST)) skip(`no urb-gateway at ${RUST} (cargo build --release -p urb-gateway)`);
// ------------------------------------------------------------ startup

// Each binary started with flags it refuses, or an identity it can't
// open: exit code and stderr must be the same, with the log's
// `2006/01/02 15:04:05 ` time and the program's path in `Usage of`
// masked. No network: each listens on 127.0.0.2 (if it gets that far)
// and fails before it opens a socket to any ship.
const results = [];
function report(name, same, a, b) {
  results.push([name, same]);
  console.log(`${same ? 'same' : 'DIFF'}  ${name}`);
  writeFileSync(join(out, `${name}.go`), a);
  writeFileSync(join(out, `${name}.rust`), b);
  if (!same) {
    const d = spawnSync('diff', ['-u', '--label', 'go', '--label', 'rust', join(out, `${name}.go`), join(out, `${name}.rust`)], { encoding: 'utf8' });
    console.log(d.stdout.split('\n').slice(0, 40).map(l => `      ${l}`).join('\n'));
  }
}

const stamp = t => t.replace(/^\d{4}\/\d\d\/\d\d \d\d:\d\d:\d\d /gm, '<time> ').replace(/^Usage of .*:$/m, 'Usage of <prog>:');
{
  const st = join(out, 'startup');
  mkdirSync(st, { recursive: true });
  // a fake moon's keyfile, as a moon's -moon keyfile, whose state here
  // is empty
  const kf = spawnSync('node', ['--input-type=module', '-e',
    `import {fakeKeyfile} from '${join(root, 'wasm/mesa.mjs')}'; import {readFileSync} from 'node:fs';
     console.log(await fakeKeyfile(readFileSync('${join(root, 'wasm/build/mesa.wasm')}'), '${MOON}'))`], { encoding: 'utf8' });
  writeFileSync(join(st, 'moon.key'), kf.stdout);
  writeFileSync(join(st, 'junk.key'), 'not a keyfile\n');
  const listen = ['-listen', `127.0.0.2:${PORT}`];
  const START = [
    ['start-fake-and-moon', ['-fake', '~zod', '-moon', 'x']],
    ['start-unknown-flag', ['-nope']],
    ['start-help', ['-h']],
    ['start-missing-value', ['-listen']],
    ['start-bad-bool', ['-new-moon=maybe']],
    ['start-no-port', ['-listen', 'nocolon']],
    ['start-fake-not-a-ship', [...listen, '-fake', 'notaship']],
    ['start-moon-missing', [...listen, '-moon', '/nonexistent/moon.key']],
    ['start-moon-directory', [...listen, '-moon', '/tmp']],
    ['start-moon-junk', [...listen, '-moon', join(st, 'junk.key')]],
    ['start-moon-no-state', [...listen, '-moon', join(st, 'moon.key')]],
  ];
  for (const [name, args] of START) {
    if (!want(name)) continue;
    const one = side => {
      const r = spawnSync(side === 'go' ? GO : RUST, ['-state', join(st, `${name}-${side}`), ...args], { encoding: 'utf8', timeout: 30_000 });
      return `exit ${r.status ?? r.signal}\n${stamp(r.stderr.split(join(st, `${name}-${side}`)).join('<state>'))}`;
    };
    const [a, b] = [one('go'), one('rust')];
    report(name, a === b, a, b);
  }
}
if (offlineOnly) finish();

const fleetOk = existsSync(FLEET) && WHO
  && spawnSync(FLEET, ['ls'], { encoding: 'utf8' }).stdout.split('\n').some(l => l.startsWith('furum ') && l.includes(`(${WHO})`));
if (!fleetOk) skip('the furum set is not claimed by FLEET_WHO, or fleet is missing');

const sleep = ms => new Promise(r => setTimeout(r, ms));

// ------------------------------------------------------------ the list

// [name, method, host (null: no Host header), target]. One connection
// each, Connection: close. ~hus runs lattice 81 with the m1c-traces
// pages; ~rup runs no lattice; ~fes runs nothing at all (30 s each side).
const h = ship => `${ship}.urb.localhost:${PORT}`;
const L = `localhost:${PORT}`;
const LIST = [
  ['home', 'GET', L, '/'],
  ['home-urb-localhost', 'GET', `urb.localhost:${PORT}`, '/'],
  ['home-127', 'GET', '127.0.0.1', '/any/path'],
  ['home-head', 'HEAD', L, '/'],
  ['go', 'GET', L, '/go?url=urb://~hus/fleet-check'],
  ['go-trimmed-non-ascii', 'GET', L, '/go?url=%20urb%3A%2F%2F~hus%2Fa%C3%A9b%22%3C%20'],
  ['go-head', 'HEAD', L, '/go?url=urb://~hus/x'],
  ['go-not-urb', 'GET', L, '/go?url=https://example.com/'],
  ['go-no-ship', 'GET', L, '/go?url=urb://~notaship/x'],
  ['go-none', 'GET', L, '/go'],
  ['go-semicolon', 'GET', L, '/go?url=urb://~hus/x;y'],
  ['go-bad-escape', 'GET', L, '/go?url=%zzurb://~hus/x'],
  ['go-second-value', 'GET', L, '/go?x=1&url=urb://~hus/a&url=urb://~hus/b'],
  ['post', 'POST', h('hus'), '/fleet-check'],
  ['put-home', 'PUT', L, '/'],
  ['unknown-host', 'GET', 'evil.example', '/'],
  ['subdomain', 'GET', h('a.hus'), '/'],
  ['not-a-ship', 'GET', h('notaship'), '/'],
  ['upper-case-ship', 'GET', h('HUS'), '/'],
  ['hus-index', 'GET', h('hus'), '/'],
  ['fleet-check', 'GET', h('hus'), '/fleet-check'],
  ['fleet-check-raw', 'GET', h('hus'), '/fleet-check?raw'],
  ['fleet-check-raw-value', 'GET', h('hus'), '/fleet-check?raw=0&x=1'],
  ['fleet-check-head', 'HEAD', h('hus'), '/fleet-check'],
  ['fleet-check-slash', 'GET', h('hus'), '/fleet-check/'],
  ['fleet-check-n', 'GET', h('hus'), '/n/fleet-check'],
  ['n-mount-empty', 'GET', h('hus'), '/n'],
  ['fleet-check-no-port', 'GET', 'hus.urb.localhost', '/fleet-check'],
  ['big', 'GET', h('hus'), '/big'],
  ['big-raw', 'GET', h('hus'), '/big?raw'],
  ['marks-md', 'GET', h('hus'), '/marks/md'],
  ['marks-html', 'GET', h('hus'), '/marks/html'],
  ['marks-text', 'GET', h('hus'), '/marks/text'],
  ['marks-gmi', 'GET', h('hus'), '/marks/gmi'],
  ['marks-css', 'GET', h('hus'), '/marks/css'],
  ['marks-encoded-slash', 'GET', h('hus'), '/marks%2Fmd'],
  ['stale-id', 'GET', h('hus'), '/m1c-stale'],
  ['missing', 'GET', h('hus'), '/no-such-page'],
  ['missing-under', 'GET', h('hus'), '/marks'],
  ['know', 'GET', h('hus'), '/k/fleet/withheld'],
  ['tree', 'GET', h('hus'), '/t/apps/x'],
  ['live', 'GET', h('hus'), '/p/counter'],
  ['not-a-mount', 'GET', h('hus'), '/x/foo'],
  ['bad-path', 'GET', h('hus'), '/Caps'],
  ['space', 'GET', h('hus'), '/a%20b'],
  ['empty-segment', 'GET', h('hus'), '/a//b'],
  ['forum-board', 'GET', h('hus'), '/forum/m1btest'],
  ['forum-post', 'GET', h('hus'), '/forum/m1btest/1'],
  ['forum-page', 'GET', h('hus'), '/forum'],
  ['no-lattice', 'GET', h('rup'), '/'],
  ['no-lattice-page', 'GET', h('rup'), '/page'],
  ['offline', 'GET', h('fes'), '/page'],
  ['fleet-check-again', 'GET', h('hus'), '/fleet-check'],
];
// requests written raw: most are refused by Go's server before its
// handler runs, and those answers carry no Date, so they are compared
// whole; the ones marked 'handled' reach the handler, and are compared
// as LIST's are
const RAW = [
  ['raw-no-host', 'GET / HTTP/1.1\r\nConnection: close\r\n\r\n'],
  ['raw-http10-no-host', 'GET / HTTP/1.0\r\n\r\n', 'handled'],
  ['raw-http10-home', `GET / HTTP/1.0\r\nHost: ${L}\r\n\r\n`, 'handled'],
  ['raw-bad-host', `GET / HTTP/1.1\r\nHost: hus.urb.localhost:${PORT}"\r\nConnection: close\r\n\r\n`],
  ['raw-two-hosts', `GET / HTTP/1.1\r\nHost: ${L}\r\nHost: ${L}\r\nConnection: close\r\n\r\n`],
  ['raw-bad-escape', `GET /fleet%zz HTTP/1.1\r\nHost: ${h('hus')}\r\nConnection: close\r\n\r\n`],
  ['raw-absolute', `GET http://${h('hus')}/fleet-check HTTP/1.1\r\nHost: ${L}\r\nConnection: close\r\n\r\n`, 'handled'],
  ['raw-http2', `GET / HTTP/2.0\r\nHost: ${L}\r\n\r\n`],
  ['raw-bad-line', 'GARBAGE\r\n\r\n'],
];

// ------------------------------------------------------------ HTTP

// one request on a connection of its own: the raw bytes of the answer
function ask(ip, bytes) {
  return new Promise(resolve => {
    const chunks = [];
    const s = connect({ host: ip, port: Number(PORT) }, () => s.end(bytes));
    s.setTimeout(90_000, () => s.destroy());
    s.on('data', d => chunks.push(d));
    s.on('close', () => resolve(Buffer.concat(chunks)));
    s.on('error', () => {});
  });
}

const request = ([, method, host, target]) =>
  `${method} ${target} HTTP/1.1\r\n${host === null ? '' : `Host: ${host}\r\n`}Connection: close\r\n\r\n`;

// an answer read: its status line, its header lines, and its body
// (de-chunked; none for a HEAD)
function parse(raw, head) {
  const end = raw.indexOf('\r\n\r\n');
  if (end < 0) return { status: raw.toString('latin1'), headers: [], body: Buffer.alloc(0) };
  const lines = raw.subarray(0, end).toString('latin1').split('\r\n');
  const status = lines.shift();
  let body = raw.subarray(end + 4);
  if (lines.some(l => /^transfer-encoding: *chunked$/i.test(l))) {
    const parts = [];
    let at = 0;
    for (;;) {
      const nl = body.indexOf('\r\n', at);
      const n = parseInt(body.subarray(at, nl).toString('latin1'), 16);
      if (!n) break;
      parts.push(body.subarray(nl + 2, nl + 2 + n));
      at = nl + 2 + n + 2;
    }
    body = Buffer.concat(parts);
  }
  if (head) body = Buffer.alloc(0);
  return { status, headers: lines, body };
}

// The normalisation, the only one, and why:
//   - Date: the wall clock;
//   - Content-Length, Transfer-Encoding and Connection: framing. Go
//     chunks a body longer than its 2 KB buffer, and says Connection:
//     close as it closes; the Rust gateway always sends a length. The
//     body is compared de-chunked, byte for byte, which fixes its
//     length, and the connection is closed after each answer on both
//     sides.
// The status line, every other header (name, value and order) and the
// body are compared as they came. The refused requests (RAW) are
// compared whole, with nothing dropped.
const FRAMING = /^(date|content-length|transfer-encoding|connection):/i;
const shown = (p, raw) => raw
  ? p.toString('latin1')
  : `${p.status}\n${p.headers.filter(l => !FRAMING.test(l)).join('\n')}\n\n${p.body.toString('latin1')}`;

// ------------------------------------------------------------ the runs

function reset() {
  const r = spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`fleet reset: ${r.stderr}`);
}

// a gateway running until stopped: its log, and stop() (Ctrl-C)
async function start(side, args) {
  const bin = side === 'go' ? GO : RUST;
  const p = spawn(bin, [...args, '-listen', `${AT[side]}:${PORT}`]);
  let log = '';
  p.stdout.on('data', d => { log += d; });
  p.stderr.on('data', d => { log += d; });
  const exited = new Promise(r => p.on('exit', (c, s) => r(c ?? `signal ${s}`)));
  const t = Date.now();
  while (!log.includes('serving http://') && p.exitCode === null && Date.now() - t < 60_000) await sleep(100);
  if (!log.includes('serving http://')) throw new Error(`${side} did not start: ${log}`);
  return {
    log: () => log,
    async stop() {
      p.kill('SIGINT');
      return Promise.race([exited, sleep(15_000).then(() => { p.kill('SIGKILL'); return 'hung'; })]);
    },
  };
}

async function answers(side) {
  const got = {};
  for (const c of LIST) {
    if (!want(c[0])) continue;
    got[c[0]] = parse(await ask(AT[side], request(c)), c[1] === 'HEAD');
    await sleep(200);
  }
  for (const [name, bytes, handled] of RAW) {
    if (!want(name)) continue;
    const a = await ask(AT[side], bytes);
    got[name] = handled ? parse(a, false) : a;
    await sleep(200);
  }
  return got;
}

// whether a UDP port is bound on 127.0.0.1 (/proc/net/udp)
const udpBound = port => readFileSync('/proc/net/udp', 'utf8').split('\n')
  .some(l => l.trim().split(/\s+/)[1] === `0100007F:${Number(port).toString(16).toUpperCase().padStart(4, '0')}`);


const goState = join(out, 'state-go'), rustState = join(out, 'state-rust');
const fakeArgs = state => ['-fake', MOON, '-state', state];
const base = `fake-${MOON.slice(1)}`;

reset();
console.log(`Go gateway as ${MOON}, fresh state in ${goState}`);
const g = await start('go', fakeArgs(goState));
const goAns = await answers('go');
console.log(`  stopped: ${await g.stop()}`);
writeFileSync(join(out, 'go.log'), g.log());

// the Go gateway's folder, copied as it left it (its .db, -wal, -shm,
// .port and .lock)
cpSync(goState, rustState, { recursive: true });
const port = readFileSync(join(rustState, `${base}.db.port`), 'utf8').trim();
console.log(`Rust gateway as ${MOON}, on the copy in ${rustState} (UDP port ${port})`);
const r = await start('rust', fakeArgs(rustState));
const portKept = udpBound(port);
const rustAns = await answers('rust');
console.log(`  stopped: ${await r.stop()}`);
writeFileSync(join(out, 'rust.log'), r.log());

for (const c of [...LIST, ...RAW]) {
  const name = c[0];
  if (!want(name)) continue;
  const raw = RAW.includes(c) && !c[2];
  const [a, b] = [shown(goAns[name], raw), shown(rustAns[name], raw)];
  report(name, a === b, a, b);
}
report('udp-port-kept', portKept && readFileSync(join(rustState, `${base}.db.port`), 'utf8').trim() === port,
  `bound ${port}\n`, `${portKept ? 'bound' : 'not bound'} ${readFileSync(join(rustState, `${base}.db.port`), 'utf8').trim()}\n`);

// the control: the same moon from an empty folder gets no answer from
// ~hus, so the reads above did continue the Go gateway's state
if (want('control')) {
  const s = await start('rust', fakeArgs(join(out, 'state-control')));
  const p = parse(await ask(AT.rust, request(['', 'GET', h('hus'), '/fleet-check'])), false);
  await s.stop();
  const ok = p.status !== 'HTTP/1.1 200 OK';
  console.log(`${ok ? 'ok  ' : 'FAIL'}  control: an empty folder as ${MOON} gets ${p.status}`);
  results.push(['control', ok]);
}

reset();

// the comet: a Go-written comet.key (mined offline), its <comet>.db and
// port file written by the Go gateway, then opened by the Rust gateway
// from a copy; each answers its home page, which names the comet. In a
// network namespace with nothing but loopback, so nothing leaves.
if (want('comet') && spawnSync('unshare', ['-rn', 'true']).status === 0) {
  const dir = join(out, 'comet');
  mkdirSync(join(dir, 'go'), { recursive: true });
  const mined = spawnSync('go', ['run', './core/ci/cometkey', join(dir, 'go', 'comet.key')], { cwd: root, encoding: 'utf8' });
  if (mined.status !== 0) throw new Error(`cometkey: ${mined.stderr}`);
  const comet = mined.stdout.trim();
  const script = `
    set -e
    ip link set lo up
    ip addr add 127.0.0.2/8 dev lo 2>/dev/null || true
    one() { "$1" -state "$2" -listen "$3:${PORT}" > "$2.log" 2>&1 & pid=$!
      for i in $(seq 50); do grep -q 'serving http' "$2.log" && break; sleep 0.1; done
      curl -s -i -H 'Host: ${L}' "http://$3:${PORT}/" > "$2.home"
      p=$(cat "$2"/*.db.port); grep -qi ":$(printf '%04X' "$p") " /proc/net/udp && echo "bound $p" > "$2.port" || echo "not bound $p" > "$2.port"
      kill -INT $pid; wait $pid || true; }
    one '${GO}' '${dir}/go' 127.0.0.2
    cp -a '${dir}/go' '${dir}/rust'
    one '${RUST}' '${dir}/rust' 127.0.0.3
  `;
  const ns = spawnSync('unshare', ['-rn', 'sh', '-c', script], { encoding: 'utf8' });
  if (ns.status !== 0) throw new Error(`comet case: ${ns.stderr}`);
  const home = side => shown(parse(readFileSync(join(dir, `${side}.home`)), false), false);
  report('comet-home', home('go') === home('rust') && home('go').includes(comet), home('go'), home('rust'));
  const portOf = side => readFileSync(join(dir, `${side}.port`), 'utf8');
  report('comet-port', portOf('go') === portOf('rust') && portOf('go').startsWith('bound'), portOf('go'), portOf('rust'));
  const dbs = side => spawnSync('ls', [join(dir, side)], { encoding: 'utf8' }).stdout.split('\n').filter(f => /\.db(\.port)?$|^comet\.key$/.test(f)).join(' ');
  report('comet-files', dbs('go') === dbs('rust'), `${dbs('go')}\n`, `${dbs('rust')}\n`);
}

finish();

function finish() {
  const bad = results.filter(x => !x[1]).map(x => x[0]);
  console.log(`\ngateway-diff: ${results.length - bad.length} of ${results.length} the same${bad.length ? `; differ: ${bad.join(', ')}` : ''}\n(outputs in ${out})`);
  process.exit(bad.length ? 1 : 0);
}
