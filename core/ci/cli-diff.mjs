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
// side, which then runs as the same moon. Exit code, stdout and stderr
// are compared byte for byte once masked; mask() and normErr() list each
// mask and why. A read-only case is retried only for ~hus's flake (no
// answer at all), and every retry is printed and counted.

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
  // ~hus's web server can take a few seconds more than the reset to
  // answer: ship-cookie is tried up to 5 times, each wait printed
  let c;
  for (let i = 1; ; i++) {
    c = spawnSync(COOKIE, [`${process.env.HOME}/fleet/piers/hus`, HUS, jar], { encoding: 'utf8' });
    if (c.status === 0 || i >= 5) break;
    console.log(`waiting for ~hus's web server (${i}): ${c.stderr.trim()}`);
    spawnSync('sleep', ['5']);
  }
  if (c.status !== 0) throw new Error(`ship-cookie: ${c.stderr}`);
  const line = readFileSync(jar, 'utf8').split('\n').find(l => l.includes('urbauth-'));
  const f = line.split('\t');
  cookie = `${f[5]}=${f[6]}`;
}

// What differs from run to run, masked, the same on both sides. Each
// normalisation, and why (docs/rust-core.md, M7's findings, lists them
// too):
//   stdout and stderr:
//   - our own ship's name, where the sides differ (`us`): read-only
//     cases run each side as its own fake moon, so the name differs;
//     likewise the state folder, in the cases that print it;
//   - lattice's page ids (0v…, 20 digits or more): random per page;
//   - in cases that write (`dates`), the minute of a new post: the
//     two sides post minutes apart.
//   stderr only:
//   - the time at the start of a log line (`mm:ss.sss `): wall clock;
//   - durations (`1.2s`): wall clock;
//   - Node's printing of an error's stack, which the native command
//     line leaves out by design: an uncaught error's source line block
//     (`file:///…:N` or `node:…:N`, the source line, the caret, a blank
//     line), the `    at …` frames, the `{ code: … }` block after them
//     and the closing `Node.js vN.N.N`. The error's own line is kept.
// Nothing else is dropped: stdout is compared byte for byte once masked.
function mask(text, us, dates) {
  for (const u of us) text = text.split(u).join('<us>');
  text = text.replace(/0v[0-9a-v.]{20,}/g, '<id>');
  if (dates) text = text.replace(/\b\d{4}-\d\d-\d\dT\d\d:\d\d\b/g, '<date>');
  return text;
}
function normErr(text, us, dates) {
  const ls = text.split('\n'), kept = [];
  let frames = false;
  for (let i = 0; i < ls.length; i++) {
    let l = ls[i];
    if (/^(file:\/\/\/|node:)\S+:\d+$/.test(l) && /^\s*\^+\s*$/.test(ls[i + 2] ?? '') && ls[i + 3] === '') {
      i += 3;
      continue;
    }
    if (/^    at /.test(l)) { frames = true; continue; }
    if (frames && (/^  [\w$]+: /.test(l) || l === '}')) continue;
    frames = false;
    if (/^Node\.js v\d+\.\d+\.\d+$/.test(l)) {
      if (kept.at(-1) === '') kept.pop();
      continue;
    }
    if (/^\d\d:\d\d\.\d\d\d /.test(l)) l = l.slice(10);
    kept.push(l.replace(/\b\d+\.\ds\b/g, '<t>s'));
  }
  return mask(kept.join('\n'), us, dates);
}

// ~hus's flake (M7's findings): no answer for a new moon within the
// base probes. Only this is retried, never a content difference.
const flaked = x => /answered no lattice peek/.test(x[2].trimEnd().split('\n').at(-1));

const results = [], retries = [];
function compare(name, n, r, us, dates = false) {
  const show = x => `exit ${x[0]}\n--- stdout\n${mask(x[1], us, dates)}\n--- stderr\n${normErr(x[2], us, dates)}\n`;
  const [a, b] = [show(n), show(r)];
  writeFileSync(join(out, `${name}.node`), `${a}\n=== raw stdout\n${n[1]}\n=== raw stderr\n${n[2]}`);
  writeFileSync(join(out, `${name}.rust`), `${b}\n=== raw stdout\n${r[1]}\n=== raw stderr\n${r[2]}`);
  return [a === b, a, b];
}
function report(name, [same, a, b]) {
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
const check = (name, n, r, us, dates) => report(name, compare(name, n, r, us, dates));

const want = name => !only.size || only.has(name);
const state = (name, side) => join(out, 'state', `${name}-${side}`);

// ------------------------------------------------------------ no ship

const offline = [
  ['read-usage', 'read', []],
  ['read-unknown-option', 'read', ['--nope', 'urb://~hus']],
  ['read-missing-value', 'read', ['urb://~hus', '--tries']],
  ['read-moon-missing', 'read', ['--moon', '/nonexistent/moon.key', 'urb://~hus']],
  ['read-moon-directory', 'read', ['--moon', '/tmp', 'urb://~hus']],
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
  check(name, run('node', cmd, args, state(name, 'node')), run('rust', cmd, args, state(name, 'rust')), []);
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
  check('read-relay-down', run('node', 'read', args, sn), run('rust', 'read', args, sr), []);
}

// pages.json can't be saved (its .new, or itself, a directory), and a
// page's file is a directory: put fails in writeFileSync's, renameSync's
// and readFileSync's words. A fake galaxy that is no fleet ship's.
for (const [name, block] of [['serve-save-fails-open', 'pages.json.new'],
  ['serve-save-fails-rename', 'pages.json/x'], ['serve-put-directory', null]]) {
  if (!want(name)) continue;
  const [sn, sr] = [state(name, 'node'), state(name, 'rust')];
  for (const s of [sn, sr]) mkdirSync(join(s, 'fake-nec', block ?? ''), { recursive: true });
  writeFileSync(join(out, 'page.md'), '# a page\n');
  const args = ['--fake', '~nec', '--put', `notes=${block ? join(out, 'page.md') : out}`];
  check(name, run('node', 'serve', args, sn), run('rust', 'serve', args, sr), [sn, sr]);
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
    // and ours alike (docs/rust-core.md, M7's findings): a case where
    // one side got no answer at all is run once more, both sides as new
    // moons, the side that went first now second. Any other difference
    // is reported as it is. Each try keeps its own files.
    for (let tries = 1; ; tries++) {
      const [a, b] = [moon(), moon()];
      const go = side => {
        const x = run(side, cmd, ['--fake', side === 'node' ? a : b, ...args], state(`${name}-${tries}`, side));
        // a traced run's datagrams are as many as the network made them
        if (what === 'stdout') x[2] = x[2].split('\n').filter(l => / (as|ok) /.test(l)).join('\n');
        return x;
      };
      let n, r;
      if (tries % 2) { n = go('node'); r = go('rust'); } else { r = go('rust'); n = go('node'); }
      const label = tries > 1 ? `${name}-try${tries}` : name;
      const c = compare(label, n, r, [a, b]);
      if (!c[0] && tries < 2 && (flaked(n) || flaked(r))) {
        const who = [flaked(n) && `node as ${a}`, flaked(r) && `rust as ${b}`].filter(Boolean).join(', ');
        retries.push(`${name}: no answer for ${who} (first ${tries % 2 ? 'node' : 'rust'})`);
        console.log(`retry ${name}: ~hus answered no lattice peek for ${who}`);
        continue;
      }
      report(name, c);
      break;
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
  check(name, n, r, [], true);
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
  check('serve', n, r, []);
  // again, crossed: no ship can read a fake moon heard before, so only
  // what each says of the pages it finds
  const again = ['--fake', m];
  const n2 = await serveRun('node', again, sr, []);
  const r2 = await serveRun('rust', again, sn, []);
  check('serve-again-crossed', n2, r2, []);
  const bad = ['--fake', m, '--put', 'BAD NAME=' + join(dir, 'hello.md')];
  check('serve-bad-name', await serveRun('node', bad, state('serve-bad', 'node'), []),
    await serveRun('rust', bad, state('serve-bad', 'rust'), []), []);
}

spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
finish();

function finish() {
  const bad = results.filter(r => !r[1]).map(r => r[0]);
  console.log(`\ncli-diff: ${results.length - bad.length} of ${results.length} the same${bad.length ? `; differ: ${bad.join(', ')}` : ''}; ${retries.length} retried${retries.map(r => `\n  retry ${r}`).join('')}\n(outputs in ${out})`);
  process.exit(bad.length ? 1 : 0);
}
