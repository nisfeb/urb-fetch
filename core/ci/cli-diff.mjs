#!/usr/bin/env node
// The differential run of M7 (docs/rust-core.md): each of today's Node
// command lines (wasm/host.mjs, pub-cli.mjs, furum-cli.mjs and
// forum-cli.mjs, on glue.c's mesa.wasm) and the native one (`urb read`,
// `serve`, `furum`, `forum`) run the same commands against the fleet,
// and their stdout, stderr and exit codes must be the same, once what
// differs from one run to the next is masked.
//
//   FLEET_WHO=lattice-loop-m7 node core/ci/cli-diff.mjs [--out DIR] [NAME...]
//   node core/ci/cli-diff.mjs --offline      the cases that need no ship
//
// It needs the furum fleet set claimed by FLEET_WHO and its m1c-traces
// snapshot (traces/fixtures.sh), wasm/build/mesa.wasm, the urb binary
// (URB, default core/target/release/urb) and
// hoon-test-kit's ship-cookie.sh (COOKIE). Without them it says what is
// missing and exits 2, a skip, never a pass. Cases that need no ship run
// first, and run even without the fleet. It leaves the set on the
// snapshot.
//
// The fleet teaches two things. An identity can be used once per
// snapshot on the fake network, so read-only cases run on one reset,
// each side as its own fake moon, the name masked; a case that writes
// (a post, a comment, a vote, a page served) resets the set before each
// side, which then runs as the same moon. What is masked: the time at
// the start of each log line, durations (`1.2s`), our own ship's name
// where the two sides differ, lattice's random page ids (0v…), and, in
// cases that write, the minute a new post was made. Node's stack traces
// are dropped, keeping the error's own line.

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const FLEET = process.env.FLEET ?? `${process.env.HOME}/software/personal/hoon-development/fleet/fleet`;
const COOKIE = process.env.COOKIE ?? `${process.env.HOME}/software/personal/furum/scripts/hoon-test-kit/ship-cookie.sh`;
const URB = process.env.URB ?? join(root, 'core/target/release/urb');
const WHO = process.env.FLEET_WHO;
const SNAP = 'm1c-traces';
const HUS = 'http://localhost:8134';

const argv = process.argv.slice(2);
let out = null, offlineOnly = false;
if (argv[0] === '--offline') { offlineOnly = true; argv.shift(); }
if (argv[0] === '--out') { out = argv[1]; argv.splice(0, 2); }
out ??= mkdtempSync(join(tmpdir(), 'cli-diff-'));
mkdirSync(out, { recursive: true });
const only = new Set(argv);

const skip = why => { console.error(`cli-diff: SKIPPED, not passed: ${why}`); process.exit(2); };
if (!existsSync(URB)) skip(`no urb binary at ${URB} (cargo build --release -p urb-cli)`);
if (!existsSync(join(root, 'wasm/build/mesa.wasm'))) skip('no wasm/build/mesa.wasm (wasm/build.sh)');

// moons of ~hus (and of ~rup, to serve: ~hus reaches a moon through its
// running parent), a few for each side. ~natdul-molwyl-dozzod-dozhus is
// left out: on m1c-traces ~hus reads it a board but never a thread, for
// the Node and the native command lines alike.
const SUFFIXES = 'dur wep ser sun ryp syx dyr nup heb peg lup dep dys put lug hec ryt tyv syd nex lun mep lut sep pes del sul ped tem led tul met wen byn hex feb pyl dul het mev rut tyl wyd tep bes dex sef wyc bur der nep pur rys reb den nut sub pet rul syn reg tyd sup sem wyn rec meg net sec mul nym tev web sum mut nyx rex teb fus hep ben mus wyx sym sel ruc dec wex syr wet dyl myn mes det bet bel tux tug myr pel syp ter meb set dut deg tex sur fel tud nux rux ren wyt nub med lyt dus neb rum tyn seg lyx pun res red fun rev ref mec ted rus bex leb dux ryn num pyx ryg ryx fep tyr tus tyc leg nem fer mer ten lus nus syl tec mex pub rym tuc fyl lep deb ber mug hut tun byl sud pem dev lur def bus bep run mel pex dyt byt typ lev myl wed duc fur fex nul luc len ner lex rup ned lec ryd lyd fen wel nyd hus rel rud nes hes fet des ret dun ler nyr seb hul ryl lud rem lys fyn wer ryc sug nys nyl lyn dyn dem lux fed sed bec mun lyr tes mud nyt byr sen weg fyr mur tel rep teg pec nel nev fes'.split(' ');
let moons = 0;
const moon = (parent = 'hus') => {
  if (moons >= SUFFIXES.length) throw new Error('out of moons');
  return `~natdul-mol${SUFFIXES[moons++]}-dozzod-doz${parent}`;
};

const NODE = { read: 'host.mjs', serve: 'pub-cli.mjs', furum: 'furum-cli.mjs', forum: 'forum-cli.mjs' };

// one run: [code, stdout, stderr]
function run(side, cmd, args, state) {
  const a = ['--state', state, ...args];
  const r = side === 'node'
    ? spawnSync('node', [join(root, 'wasm', NODE[cmd]), ...a], { encoding: 'utf8', timeout: 300_000 })
    : spawnSync(URB, [cmd, ...a], { encoding: 'utf8', timeout: 300_000 });
  return [r.status ?? `signal ${r.signal}`, r.stdout ?? '', r.stderr ?? ''];
}

// a server run: started, read by ~hus's lattice while it serves, then
// Ctrl-C
async function serveRun(side, args, state, reads) {
  const a = ['--state', state, ...args];
  const p = side === 'node'
    ? spawn('node', [join(root, 'wasm/pub-cli.mjs'), ...a])
    : spawn(URB, ['serve', ...a]);
  let so = '', se = '';
  p.stdout.on('data', d => { so += d; });
  p.stderr.on('data', d => { se += d; });
  const exited = new Promise(r => p.on('exit', (c, s) => r(c ?? `signal ${s}`)));
  const t = Date.now();
  while (!se.includes('serving; Ctrl-C stops') && Date.now() - t < 60_000 && p.exitCode === null) await sleep(100);
  const got = [];
  if (se.includes('serving;')) {
    for (const url of reads) {
      got.push(await husFetch(url));
      await sleep(1500);
    }
    await sleep(1000);
  }
  p.kill('SIGINT');
  const code = await Promise.race([exited, sleep(15_000).then(() => { p.kill('SIGKILL'); return 'hung'; })]);
  return [code, so + got.map(g => `~hus read: ${g}\n`).join(''), se];
}

const sleep = ms => new Promise(r => setTimeout(r, ms));

let cookie = null;
async function husFetch(url) {
  try {
    const r = await fetch(`${HUS}/apps/lattice/fetch?url=${encodeURIComponent(url)}`,
      { headers: { cookie }, signal: AbortSignal.timeout(90_000) });
    return `${r.status} ${await r.text()}`;
  } catch (e) {
    return `failed: ${e.message}`;
  }
}

function reset() {
  const r = spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`fleet reset: ${r.stderr}`);
  const jar = join(out, 'hus.jar');
  const c = spawnSync(COOKIE, [`${process.env.HOME}/fleet/piers/hus`, HUS, jar], { encoding: 'utf8' });
  if (c.status !== 0) throw new Error(`ship-cookie: ${c.stderr}`);
  const line = readFileSync(jar, 'utf8').split('\n').find(l => l.includes('urbauth-'));
  const f = line.split('\t');
  cookie = `${f[5]}=${f[6]}`;
}

// what differs from run to run, masked. Of the lines without a time,
// only an error's own line (and a Rust panic's) is kept: the rest is
// Node's printing of an uncaught error, its source line and stack.
function norm(text, us, dates) {
  const lines = [];
  for (let l of text.split('\n')) {
    if (/^\d\d:\d\d\.\d\d\d /.test(l)) l = l.slice(10);
    else if (!/^\w*Error( \[\w+\])?: /.test(l) && !/^thread '/.test(l)) continue;
    for (const u of us) l = l.split(u).join('<us>');
    l = l.replace(/\b\d+\.\ds\b/g, '<t>s').replace(/0v[0-9a-v.]{20,}/g, '<id>');
    if (dates) l = l.replace(/\b\d{4}-\d\d-\d\dT\d\d:\d\d\b/g, '<date>');
    lines.push(l);
  }
  return lines.join('\n');
}

const results = [];
// the same? A first try that may be retried is reported as such, and
// not counted
function compare(name, n, r, us, dates = false, again = false) {
  const show = x => `exit ${x[0]}\n--- stdout\n${norm(x[1], us, dates)}\n--- stderr\n${norm(x[2], us, dates)}\n`;
  const [a, b] = [show(n), show(r)];
  writeFileSync(join(out, `${name}.node`), `${a}\n=== raw stderr\n${n[2]}`);
  writeFileSync(join(out, `${name}.rust`), `${b}\n=== raw stderr\n${r[2]}`);
  const same = a === b;
  if (!same && again) {
    console.log(`retry ${name}: ${norm(n[2], us).split('\n').at(-1)} | ${norm(r[2], us).split('\n').at(-1)}`);
    return false;
  }
  results.push([name, same]);
  console.log(`${same ? 'same' : 'DIFF'}  ${name}`);
  if (!same) {
    writeFileSync(join(out, 'a'), a);
    writeFileSync(join(out, 'b'), b);
    const d = spawnSync('diff', ['-u', '--label', 'node', '--label', 'rust', join(out, 'a'), join(out, 'b')], { encoding: 'utf8' });
    console.log(d.stdout.split('\n').map(l => `      ${l}`).join('\n'));
  }
  return same;
}

const want = name => !only.size || only.has(name);
const state = (name, side) => join(out, 'state', `${name}-${side}`);

// ------------------------------------------------------------ no ship

const offline = [
  ['read-usage', 'read', []],
  ['read-unknown-option', 'read', ['--nope', 'urb://~hus']],
  ['read-missing-value', 'read', ['urb://~hus', '--tries']],
  ['read-moon-missing', 'read', ['--moon', '/nonexistent/moon.key', 'urb://~hus']],
  ['read-fake-and-relay', 'read', ['--fake', '~hus', '--relay', 'ws://localhost:1', 'urb://~hus']],
  ['read-fake-not-a-ship', 'read', ['--fake', '~natdul-molfab-dozzod-dozhus', 'urb://~hus']],
  ['furum-usage', 'furum', ['~rup']],
  ['forum-usage', 'forum', ['urb://~hus/forum/m1btest', '--upvote']],
  ['forum-not-forum', 'forum', ['urb://~hus/page']],
  ['serve-positional', 'serve', ['extra']],
  ['furum-fake-and-relay', 'furum', ['--fake', '~hus', '--relay', 'ws://x', '~rup', 'm1c']],
];
// a keyfile that is not a moon's: a fake galaxy's
{
  const kf = spawnSync('node', ['--input-type=module', '-e',
    `import {fakeKeyfile} from '${join(root, 'wasm/mesa.mjs')}'; import {readFileSync} from 'node:fs';
     console.log(await fakeKeyfile(readFileSync('${join(root, 'wasm/build/mesa.wasm')}'), '~hus'))`], { encoding: 'utf8' });
  writeFileSync(join(out, 'galaxy.key'), kf.stdout);
  offline.push(['read-moon-not-a-moon', 'read', ['--moon', join(out, 'galaxy.key'), 'urb://~hus']]);
}
for (const [name, cmd, args] of offline) {
  if (!want(name)) continue;
  compare(name, run('node', cmd, args, state(name, 'node')), run('rust', cmd, args, state(name, 'rust')), []);
}
// a comet whose relay is down: open fails as the transport's start does
// (trace 1's comet, so nothing is mined from the live bootstrap)
if (want('read-relay-down')) {
  const key = JSON.parse(readFileSync(join(root, 'traces/1-cold-comet.jsonl'), 'utf8').split('\n')
    .find(l => l.includes('"saveKey"'))).out.find(o => o.saveKey).saveKey;
  const args = ['--relay', 'ws://127.0.0.1:1', 'urb://~hus'];
  const [sn, sr] = [state('read-relay-down', 'node'), state('read-relay-down', 'rust')];
  for (const s of [sn, sr]) {
    mkdirSync(s, { recursive: true });
    writeFileSync(join(s, 'comet.key'), `${key}\n`);
  }
  compare('read-relay-down', run('node', 'read', args, sn), run('rust', 'read', args, sr), []);
}

// ------------------------------------------------------------ the fleet

if (offlineOnly) finish();

const fleetOk = existsSync(FLEET) && existsSync(COOKIE) && WHO
  && spawnSync(FLEET, ['ls'], { encoding: 'utf8' }).stdout.split('\n').some(l => l.startsWith('furum ') && l.includes(`(${WHO})`));
if (!fleetOk) {
  console.error('cli-diff: the fleet cases were SKIPPED: the furum set is not claimed by FLEET_WHO, or fleet or ship-cookie.sh is missing');
  process.exit(2);
}

// read-only, on one reset: each side its own moon, its name masked
const reads = [
  ['read-page', 'read', ['urb://~hus/fleet-check']],
  ['read-home', 'read', ['urb://~hus']],
  ['read-big', 'read', ['urb://~hus/big']],
  ['read-missing', 'read', ['urb://~hus/no-such-page']],
  ['read-md', 'read', ['urb://~hus/marks/md']],
  ['read-html', 'read', ['urb://~hus/marks/html']],
  ['read-text', 'read', ['urb://~hus/marks/text']],
  ['read-tries', 'read', ['--tries', '2', 'urb://~hus/fleet-check']],
  ['read-withheld', 'read', ['urb://~hus/k/fleet/withheld']],
  ['read-traced', 'read', ['--trace', 'urb://~hus/fleet-check'], 'stdout'],
  ['furum-list', 'furum', ['~rup', 'm1c']],
  ['furum-thread', 'furum', ['~rup', 'm1c', '--thread', '0']],
  ['furum-no-board', 'furum', ['~rup', 'nope']],
  ['forum-board', 'forum', ['urb://~hus/forum/m1btest']],
  ['forum-thread', 'forum', ['urb://~hus/forum/m1btest/1']],
  ['forum-no-post', 'forum', ['urb://~hus/forum/m1btest/999']],
];
if (reads.some(r => want(r[0]))) {
  reset();
  for (const [name, cmd, args, what] of reads) {
    if (!want(name)) continue;
    // ~hus now and then answers no new moon for a few seconds, Node's
    // and ours alike (docs/rust-core.md, M7's findings): a read-only
    // case that differs is run once more, both sides as new moons
    for (let tries = 1; ; tries++) {
      const [a, b] = [moon(), moon()];
      const n = run('node', cmd, ['--fake', a, ...args], state(`${name}-${tries}`, 'node'));
      const r = run('rust', cmd, ['--fake', b, ...args], state(`${name}-${tries}`, 'rust'));
      // a traced run's datagrams are as many as the network made them
      if (what === 'stdout') {
        n[2] = n[2].split('\n').filter(l => / (as|ok) /.test(l)).join('\n');
        r[2] = r[2].split('\n').filter(l => / (as|ok) /.test(l)).join('\n');
      }
      if (compare(name, n, r, [a, b], false, tries < 2) || tries >= 2) break;
    }
  }
}

// cases that write: the set reset before each side, which runs as the
// same moon, and the minute a post was made masked
const writes = [
  ['furum-post', 'furum', ['~rup', 'm1c', '--post', 'An M7 post', '--body', 'from the differential run']],
  ['furum-comment', 'furum', ['~rup', 'm1c', '--comment', '0', '--body', 'an M7 comment']],
  ['furum-upvote', 'furum', ['~rup', 'm1c', '--upvote', '0']],
  ['forum-post', 'forum', ['urb://~hus/forum/m1btest', '--post', 'An M7 post', '--body', 'from the differential run']],
  ['forum-comment', 'forum', ['urb://~hus/forum/m1btest/1', '--comment', 'an M7 comment']],
  ['forum-upvote', 'forum', ['urb://~hus/forum/m1btest/1', '--upvote']],
];
for (const [name, cmd, args] of writes) {
  if (!want(name)) continue;
  const m = moon();
  reset();
  const n = run('node', cmd, ['--fake', m, ...args], state(name, 'node'));
  reset();
  const r = run('rust', cmd, ['--fake', m, ...args], state(name, 'rust'));
  compare(name, n, r, [], true);
}

// serving: pages put and one removed, read by ~hus's lattice; then each
// side serves again from the other's folder (its pages.json and rows)
if (want('serve')) {
  const dir = join(out, 'pages');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'hello.md'), '# Hello\n\nfrom the M7 differential run, ünïcode included\n');
  writeFileSync(join(dir, 'web.HTML'), '<p>a page of HTML</p>\n');
  writeFileSync(join(dir, 'gone.txt'), 'soon gone\n');
  writeFileSync(join(dir, 'notes'), '# no extension: gemtext\n');
  const m = moon('rup');
  const args = ['--fake', m, '--put', `notes/hello=${dir}/hello.md`, '--put', `web=${dir}/web.HTML`,
    '--put', `gone=${dir}/gone.txt`, '--put', `plain=${dir}/notes`, '--remove', 'gone', '--remove', 'never'];
  const urls = [`urb://${m}/notes/hello`, `urb://${m}/web`, `urb://${m}/gone`];
  const [sn, sr] = [state('serve', 'node'), state('serve', 'rust')];
  reset();
  const n = await serveRun('node', args, sn, urls);
  reset();
  const r = await serveRun('rust', args, sr, urls);
  compare('serve', n, r, []);
  // again, crossed: no ship can read a fake moon heard before, so only
  // what each says of the pages it finds
  const again = ['--fake', m];
  const n2 = await serveRun('node', again, sr, []);
  const r2 = await serveRun('rust', again, sn, []);
  compare('serve-again-crossed', n2, r2, []);
  const bad = ['--fake', m, '--put', 'BAD NAME=' + join(dir, 'hello.md')];
  compare('serve-bad-name', await serveRun('node', bad, state('serve-bad', 'node'), []),
    await serveRun('rust', bad, state('serve-bad', 'rust'), []), []);
}

spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
finish();

function finish() {
  const bad = results.filter(r => !r[1]).map(r => r[0]);
  console.log(`\ncli-diff: ${results.length - bad.length} of ${results.length} the same${bad.length ? `; differ: ${bad.join(', ')}` : ''} (outputs in ${out})`);
  process.exit(bad.length ? 1 : 0);
}
