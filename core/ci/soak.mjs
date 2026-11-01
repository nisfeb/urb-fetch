#!/usr/bin/env node
// Gate 5, the soak, automated (docs/rust-core.md, "Gate 5: automated
// soak"): one round, run again every few hours for a week.
//
//   node core/ci/soak.mjs [PHASE...]
//
// PHASEs, all by default, in this order: build, cli-diff, gateway-diff,
// migrate, harness, browse, gateway. A round:
//
//   - claims the fleet set furum as lattice-loop-soak. If someone else
//     holds it, it logs "skipped: claimed by X" and exits 0. If the
//     holder is the side-by-side gateway (soak-gateway.sh), which only
//     holds the set for a tick, it waits up to 10 minutes first. It
//     releases the claim at the end, on failure and on SIGINT or
//     SIGTERM too, and leaves the set reset to m1c-traces;
//   - build: the release urb and urb-gateway, and the Go urb-fetch;
//   - cli-diff, gateway-diff, migrate: the gate 2 and 3 scripts;
//   - harness: on two throwaway clones of nisfeb-extension (EXT_SRC,
//     cloned, origin removed; the source is only read), one with the
//     Rust lib/ staged by shim/build.sh and one with its own JS lib/,
//     runs npm test, npm run today, urb:fleet, the furum harness, pub
//     and marks, each fleet harness after a reset and settle, and
//     compares the two check lists word for word with ship names and
//     timings masked, as gate 4 did;
//   - browse: soak-browse.mjs, about ten minutes of normal use in a
//     throwaway headless Brave, on each clone after a reset, every page
//     body and error recorded and diffed, masked the same way;
//   - gateway: one tick of soak-gateway.sh under this round's claim.
//
// It writes /tmp/soak/round-<timestamp>/ (SOAK_DIR) with every log and
// summary.json ({ok, differences, failures, skipped, ...}), and appends
// one line to /tmp/soak/rounds.log. Exit 0 when ok or skipped, 1 when
// not. A difference is the Rust side differing from the JS or Go side;
// a failure is something that went wrong on both sides, or a phase that
// could not run. Either makes the round not ok.
//
// Tools: shim/build.sh needs WASI_SDK_PATH, and make urb-fetch ZIG. When
// they are not set they come from SOAK_TOOLS (default
// ~/.cache/urb-fetch-soak/tools), installed there by core/ci/tools.sh
// the first time.

import { spawn, spawnSync } from 'node:child_process';
import { appendFileSync, createWriteStream, existsSync, mkdirSync, readFileSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const HOME = homedir();
const FLEET = process.env.FLEET ?? `${HOME}/software/personal/hoon-development/fleet/fleet`;
const FLEET_HOME = process.env.FLEET_HOME ?? `${HOME}/fleet`;
const COOKIE = process.env.COOKIE ?? `${HOME}/software/personal/furum/scripts/hoon-test-kit/ship-cookie.sh`;
const EXT_SRC = process.env.EXT_SRC ?? `${HOME}/software/personal/nisfeb-extension`;
// SOAK_BROWSER, not BROWSER: desktops set BROWSER to their opener
const BROWSER = process.env.SOAK_BROWSER ?? '/opt/brave.com/brave/brave';
const SOAK = process.env.SOAK_DIR ?? '/tmp/soak';
const TOOLS = process.env.SOAK_TOOLS ?? `${HOME}/.cache/urb-fetch-soak/tools`;
const WHO = 'lattice-loop-soak';
const GW_WHO = 'lattice-loop-soak-gw';
const SNAP = 'm1c-traces';
const GO = join(root, 'urb-fetch');
const PHASES = ['build', 'cli-diff', 'gateway-diff', 'migrate', 'harness', 'browse', 'gateway'];

const want = process.argv.slice(2);
for (const p of want) if (!PHASES.includes(p)) { console.error(`soak: no phase ${p} (${PHASES.join(', ')})`); process.exit(2); }
const phases = want.length ? PHASES.filter(p => want.includes(p)) : PHASES;

const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
const R = join(SOAK, `round-${stamp}`);
mkdirSync(R, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));
const roundLog = createWriteStream(join(R, 'round.log'), { flags: 'a' });
const say = s => { const l = `${new Date().toISOString()} ${s}`; console.log(l); roundLog.write(`${l}\n`); };

const summary = { ok: false, round: R, started: new Date().toISOString(), commit: git('rev-parse', '--short', 'HEAD'), phases, differences: [], failures: [], notes: [], skipped: false };
function git(...a) { return spawnSync('git', ['-C', root, ...a], { encoding: 'utf8' }).stdout.trim(); }
const differ = (phase, what, detail = '') => { summary.differences.push({ phase, what, detail: String(detail).slice(0, 4000) }); say(`DIFFERENCE ${phase}: ${what}`); };
const fail = (phase, what, detail = '') => { summary.failures.push({ phase, what, detail: String(detail).slice(0, 4000) }); say(`FAILURE ${phase}: ${what}`); };

// ---------------------------------------------------------------- the claim

const holder = () => { try { return readFileSync(join(FLEET_HOME, 'claims/furum'), 'utf8').trim(); } catch { return ''; } };
const lockFile = join(SOAK, 'round.lock');
let claimed = false;
function release() {
  if (claimed && holder() === WHO) {
    spawnSync(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { encoding: 'utf8' });
    spawnSync(FLEET, ['release', 'furum']);
  }
  claimed = false;
  try { if (readFileSync(lockFile, 'utf8').trim() === String(process.pid)) writeFileSync(lockFile, ''); } catch {}
}
const children = new Set();
let stopping = false;
for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(sig, async () => {
    if (stopping) return;
    stopping = true;
    say(`soak: ${sig}, stopping`);
    for (const c of children) { try { process.kill(-c.pid, 'SIGTERM'); } catch {} }
    for (const end = Date.now() + 30_000; children.size && Date.now() < end;) await sleep(200);
    release();
    finish(`stopped by ${sig}`, 1);
  });
}

let finished = false;
function finish(why, code) {
  if (finished) return;
  finished = true;
  summary.ended = new Date().toISOString();
  if (!summary.skipped) summary.ok = summary.differences.length === 0 && summary.failures.length === 0 && !why;
  writeFileSync(join(R, 'summary.json'), JSON.stringify(summary, null, 1));
  const line = summary.skipped
    ? `${stamp} skipped: ${summary.skippedWhy}`
    : `${stamp} ${summary.ok ? 'ok' : 'NOT OK'} ${summary.differences.length} differences, ${summary.failures.length} failures${why ? `, ${why}` : ''} (${phases.join(' ')}) ${R}`;
  appendFileSync(join(SOAK, 'rounds.log'), `${line}\n`);
  say(line);
  roundLog.end(() => process.exit(code));
}

async function claim() {
  let h = holder();
  for (const end = Date.now() + 600_000; h === GW_WHO && Date.now() < end;) { await sleep(10_000); h = holder(); }
  if (h === WHO) {
    // ours by name: another round still running, or one that died
    const pid = Number((existsSync(lockFile) && readFileSync(lockFile, 'utf8').trim()) || 0);
    let alive = false;
    try { if (pid) { process.kill(pid, 0); alive = true; } } catch {}
    if (alive) return `claimed by ${WHO} (round pid ${pid})`;
    say(`soak: taking over a claim left by a round that is gone (pid ${pid || 'unknown'})`);
  } else if (h) return `claimed by ${h}`;
  spawnSync(FLEET, ['claim', 'furum', WHO]);
  await sleep(500);
  if (holder() !== WHO) return `claimed by ${holder()}`;
  writeFileSync(lockFile, String(process.pid));
  claimed = true;
  return null;
}

// ---------------------------------------------------------------- running things

// a command to a log file; resolves to its exit code
function run(cmd, args, { log, cwd = root, env = {}, timeout = 3_600_000 } = {}) {
  return new Promise(resolve => {
    const out = createWriteStream(log, { flags: 'a' });
    out.write(`$ ${cmd} ${args.join(' ')}\n`);
    const c = spawn(cmd, args, { cwd, env: { ...process.env, ...env }, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    children.add(c);
    c.stdout.pipe(out, { end: false });
    c.stderr.pipe(out, { end: false });
    const t = setTimeout(() => { out.write(`\n(timed out after ${timeout / 1000} s)\n`); try { process.kill(-c.pid, 'SIGKILL'); } catch {} }, timeout);
    c.on('close', (code, sig) => {
      clearTimeout(t);
      children.delete(c);
      out.end(`\n(exit ${code ?? `signal ${sig}`})\n`, () => resolve(code ?? `signal ${sig}`));
    });
  });
}
const tail = (f, n = 30) => { try { return readFileSync(f, 'utf8').trimEnd().split('\n').slice(-n).join('\n'); } catch { return ''; } };

async function reset(log) {
  const code = await run(FLEET, ['--as', WHO, 'reset', 'furum', SNAP], { log });
  if (code !== 0) throw new Error(`fleet reset exited ${code} (${log})`);
  const s = await run('node', ['--input-type=module', '-e', `import {settle} from '${join(root, 'core/ci/settle.mjs')}'; settle()`], { log });
  if (s !== 0) throw new Error(`settle failed (${log})`);
}

let toolEnv = {};
async function tools() {
  if (!process.env.WASI_SDK_PATH || !process.env.ZIG) {
    if (!existsSync(join(TOOLS, 'wasi-sdk.ok')) || !existsSync(join(TOOLS, 'zig.ok'))) say(`soak: installing the pinned tools into ${TOOLS}`);
    const r = spawnSync(join(root, 'core/ci/tools.sh'), [TOOLS], { encoding: 'utf8' });
    if (r.status !== 0) throw new Error(`tools.sh: ${r.stderr}`);
    for (const l of r.stdout.split('\n')) {
      const m = /^(WASI_SDK_PATH|ZIG|URBIT)=(.*)$/.exec(l);
      if (m && !process.env[m[1]]) toolEnv[m[1]] = m[2];
    }
  }
  // the wasi-sdk's clang wants libtinfo.so.6, which some systems only
  // have as ncurses' wide build: a link to it in a folder of its own
  const clang = join(toolEnv.WASI_SDK_PATH ?? process.env.WASI_SDK_PATH, 'bin/clang');
  if (/libtinfo\.so\.6 => not found/.test(spawnSync('ldd', [clang], { encoding: 'utf8' }).stdout)) {
    const lib = join(TOOLS, 'lib');
    const have = ['/usr/lib64/libncursesw.so.6', '/usr/lib/x86_64-linux-gnu/libncursesw.so.6', '/usr/lib/libncursesw.so.6'].find(existsSync);
    if (!have) throw new Error(`${clang} needs libtinfo.so.6, and there is no libncursesw.so.6 to stand in`);
    mkdirSync(lib, { recursive: true });
    if (!existsSync(join(lib, 'libtinfo.so.6'))) symlinkSync(have, join(lib, 'libtinfo.so.6'));
    toolEnv.LD_LIBRARY_PATH = [lib, process.env.LD_LIBRARY_PATH].filter(Boolean).join(':');
  }
  Object.assign(process.env, toolEnv);
}

// ---------------------------------------------------------------- masks

// what may differ from one run to the next, and nothing else: names of
// comets and moons (mined or named from the clock), times and dates,
// and how long things took
function mask(s) {
  return s
    .replace(/~[a-z]{6}(?:--?[a-z]{6})+/g, '~SHIP')
    .replace(/\b\d{4}-\d\d-\d\dT[\d:.]+Z?\b/g, 'TIME')
    .replace(/\b\d{1,2}\/\d{1,2}\/\d{4},? \d{1,2}:\d\d(:\d\d)?( ?[AP]M)?/g, 'DATE')
    .replace(/\b\d{1,2}:\d\d:\d\d\b/g, 'TIME')
    .replace(/\b\d+(\.\d+)? ?(ms|s|seconds?|minutes?)\b/g, 'N s')
    .replace(/\(\d+(\.\d+)?ms\)/g, '(N ms)')
    .replace(/duration_ms [\d.]+/g, 'duration_ms N')
    .replace(/\b\d+ (seconds?|minutes?|hours?|days?) ago\b/g, 'AGO');
}
// a harness's check list: its ok and FAIL lines (node --test's ✔ and ✖),
// its counts and its last word, masked
const CHECK = /^(ok\b|FAIL\b|not ok\b|✔|✖|ℹ (tests|suites|pass|fail|cancelled|skipped|todo)\b|all passed|\d+ failed)/;
const checks = text => text.split('\n').map(l => l.trim()).filter(l => CHECK.test(l)).map(mask);

// a unified diff of two strings, by temporary files
function diffStrings(a, b, la, lb, dir, name) {
  const fa = join(dir, `${name}.${la}`), fb = join(dir, `${name}.${lb}`);
  writeFileSync(fa, a); writeFileSync(fb, b);
  return spawnSync('diff', ['-u', '--label', la, '--label', lb, fa, fb], { encoding: 'utf8' }).stdout;
}

// ---------------------------------------------------------------- phases

async function phaseBuild() {
  const log = join(R, 'build.log');
  const env = { ...toolEnv };
  if (await run('cargo', ['build', '--release', '-p', 'urb-cli', '-p', 'urb-gateway'], { log, cwd: join(root, 'core'), env }) !== 0) throw new Error(`cargo build failed (${log})`);
  if (await run('make', ['urb-fetch'], { log, env }) !== 0) throw new Error(`make urb-fetch failed (${log})`);
  if (await run('sh', ['shim/build.sh'], { log, env }) !== 0) throw new Error(`shim/build.sh failed (${log})`);
  for (const f of ['wasm/build/mesa.wasm', 'urb-fetch', 'core/target/release/urb', 'core/target/release/urb-gateway', 'shim/build/mesa.wasm']) {
    if (!existsSync(join(root, f))) throw new Error(`no ${f} after the build`);
  }
}

async function phaseDiff(name, script, summaryRe) {
  const log = join(R, `${name}.log`);
  const code = await run('node', [join(root, 'core/ci', script), '--out', join(R, name)], { log, env: { FLEET_WHO: WHO }, timeout: 7_200_000 });
  const text = readFileSync(log, 'utf8');
  const line = (text.match(summaryRe) || [])[0] || '(no summary line)';
  summary[name] = { exit: code, result: line };
  const retried = /; (\d+) retried/.exec(text);
  if (retried && Number(retried[1]) > 0) summary.notes.push(`${name}: ${retried[1]} retried`);
  if (code === 2) fail(name, 'skipped by the script', tail(log, 5));
  else if (code !== 0) {
    const differing = /differ: (.*)/.exec(line);
    if (differing) differ(name, line, tail(log, 40));
    else fail(name, `exit ${code}: ${line}`, tail(log, 40));
  }
  say(`${name}: ${line}`);
}

function clone(dir) {
  const r = spawnSync('git', ['clone', '--quiet', '--no-hardlinks', EXT_SRC, dir], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git clone: ${r.stderr}`);
  spawnSync('git', ['-C', dir, 'remote', 'remove', 'origin']);
  return spawnSync('git', ['-C', dir, 'rev-parse', '--short', 'HEAD'], { encoding: 'utf8' }).stdout.trim();
}
async function jar(log) {
  const f = join(R, 'hus.jar');
  for (let i = 0; i < 6; i++) {
    if (await run(COOKIE, [`${FLEET_HOME}/piers/hus`, 'http://localhost:8134', f], { log, timeout: 60_000 }) === 0) return f;
    await sleep(5000);
  }
  throw new Error(`no login cookie for ~hus (${log})`);
}

let clones = null;
function clones2() {
  if (clones) return clones;
  const rust = join(R, 'ext-rust'), js = join(R, 'ext-js');
  const at = clone(rust);
  clone(js);
  const st = spawnSync('sh', [join(root, 'shim/build.sh'), join(rust, 'lib')], { cwd: root, encoding: 'utf8', env: { ...process.env, ...toolEnv } });
  writeFileSync(join(R, 'shim-stage.log'), st.stdout + st.stderr);
  if (st.status !== 0) throw new Error(`shim/build.sh ${rust}/lib failed`);
  summary.extension = at;
  say(`clones of nisfeb-extension ${at}: ${rust} (Rust lib/), ${js} (JS lib/)`);
  return (clones = { rust, js });
}

const HARNESSES = [
  ['npm-test', ['npm', 'test'], {}, false, 600_000],
  ['today', ['npm', 'run', 'today'], {}, false, 1_500_000],
  ['urb-fleet', ['node', 'scripts/urb-check.js'], { FLEET: '1' }, true, 1_500_000],
  ['furum', ['node', 'scripts/urb-check.js'], { FLEET: '1', FURUM: '~rup/m1c' }, true, 1_800_000],
  ['pub', ['node', 'scripts/pub-check.js'], { READER: 'http://localhost:8134', PARENT: '~rup' }, true, 1_500_000],
  ['marks', ['node', 'scripts/marks-check.js'], {}, true, 1_500_000],
];
async function phaseHarness() {
  const { rust, js } = clones2();
  const dir = join(R, 'harness');
  mkdirSync(dir, { recursive: true });
  const env = { BROWSER, URB_FETCH: GO };
  for (const [name, [cmd, ...args], extra, fleet, timeout] of HARNESSES) {
    const got = {};
    for (const [side, ext] of [['js', js], ['rust', rust]]) {
      const log = join(dir, `${side}-${name}.log`);
      if (fleet) await reset(join(dir, 'reset.log'));
      const e = { ...env, ...extra };
      if (name === 'pub') e.READER_JAR = await jar(join(dir, 'reset.log'));
      const code = await run(cmd, args, { log, cwd: ext, env: e, timeout });
      got[side] = { code, checks: checks(readFileSync(log, 'utf8')) };
      say(`harness ${name} on the ${side} lib/: exit ${code}, ${got[side].checks.filter(l => /^(ok|✔)/.test(l)).length} checks ok`);
    }
    const [a, b] = [got.js.checks.join('\n') + '\n', got.rust.checks.join('\n') + '\n'];
    summary.harness ??= {};
    summary.harness[name] = { js: got.js.code, rust: got.rust.code, same: a === b, checks: got.rust.checks.length };
    if (a !== b) differ('harness', `${name}: the check lists differ`, diffStrings(a, b, 'js', 'rust', dir, `${name}-checks`));
    else if (got.js.code !== got.rust.code) differ('harness', `${name}: exit ${got.js.code} on JS, ${got.rust.code} on Rust`);
    if (got.js.code !== 0 || got.rust.code !== 0) fail('harness', `${name}: exit ${got.js.code} on JS, ${got.rust.code} on Rust`, tail(join(dir, `rust-${name}.log`), 20));
  }
}

async function phaseBrowse() {
  const { rust, js } = clones2();
  const dir = join(R, 'browse');
  mkdirSync(dir, { recursive: true });
  const rec = {}, minutes = {};
  for (const [side, ext] of [['js', js], ['rust', rust]]) {
    await reset(join(dir, 'reset.log'));
    const j = await jar(join(dir, 'reset.log'));
    const t0 = Date.now();
    const code = await run('node', [join(root, 'core/ci/soak-browse.mjs'), ext, join(dir, side)],
      { log: join(dir, `${side}.log`), env: { BROWSER, SOAK_BROWSER: BROWSER, URB_FETCH: GO, READER_JAR: j }, timeout: 2_400_000 });
    minutes[side] = +((Date.now() - t0) / 60000).toFixed(1);
    rec[side] = existsSync(join(dir, side, 'steps.json')) ? JSON.parse(readFileSync(join(dir, side, 'steps.json'), 'utf8')) : [];
    say(`browse on the ${side} lib/: exit ${code}, ${rec[side].length} steps, ${rec[side].reduce((n, s) => n + s.errors.length, 0)} errors`);
    if (code !== 0) fail('browse', `the ${side} pass did not run to the end (exit ${code})`, tail(join(dir, `${side}.log`), 20));
  }
  const show = s => `status: ${mask(s.status)}\n\n${mask(s.body)}\n${s.frame === null ? '' : `\n--- frame ---\n${mask(s.frame)}\n`}\n--- errors ---\n${s.errors.map(mask).join('\n')}\n`;
  const names = [...new Set([...rec.js.map(s => s.name), ...rec.rust.map(s => s.name)])];
  let same = 0;
  for (const name of names) {
    const a = rec.js.find(s => s.name === name), b = rec.rust.find(s => s.name === name);
    if (!a || !b) { differ('browse', `step ${name} ran on the ${a ? 'JS' : 'Rust'} side only`); continue; }
    const [x, y] = [show(a), show(b)];
    if (x === y) same++;
    else differ('browse', `step ${name}: page or errors differ`, diffStrings(x, y, 'js', 'rust', dir, name));
  }
  for (const s of rec.rust) if (s.errors.length) summary.notes.push(`browse ${s.name}: ${s.errors.length} errors on the Rust side (and ${rec.js.find(x => x.name === s.name)?.errors.length ?? '?'} on the JS)`);
  summary.browse = { steps: names.length, same, minutes };
}

async function phaseGateway() {
  const log = join(R, 'gateway.log');
  const code = await run(join(root, 'core/ci/soak-gateway.sh'), ['--once', '--out', join(R, 'gateway')], { log, env: { FLEET_WHO: WHO, SOAK_DIR: SOAK } });
  const line = (readFileSync(log, 'utf8').match(/tick: .*/g) || ['(no tick)']).at(-1);
  summary.gateway = { exit: code, result: line };
  if (code === 1 && /differences/.test(line)) differ('gateway', line, `see ${join(SOAK, 'gateway-diff.log')}`);
  else if (code !== 0) fail('gateway', `exit ${code}: ${line}`, tail(log, 20));
  say(`gateway: ${line}`);
}

// ---------------------------------------------------------------- the round

const blocked = await claim();
if (blocked) {
  summary.skipped = true;
  summary.skippedWhy = blocked;
  say(`skipped: ${blocked}`);
  finish(null, 0);
} else {
  await tools().catch(e => fail('tools', e.message));
  say(`soak round ${R}: furum claimed as ${WHO}; phases ${phases.join(', ')}`);
  try {
    for (const p of phases) {
      if (stopping) break;
      const t = Date.now();
      try {
        if (p === 'build') await phaseBuild();
        if (p === 'cli-diff') await phaseDiff('cli-diff', 'cli-diff.mjs', /cli-diff: \d+ of \d+ the same.*/);
        if (p === 'gateway-diff') await phaseDiff('gateway-diff', 'gateway-diff.mjs', /gateway-diff: \d+ of \d+ the same.*/);
        if (p === 'migrate') await phaseDiff('migrate', 'migrate.mjs', /migrate: (every check passed|\d+ FAILED).*/);
        if (p === 'harness') await phaseHarness();
        if (p === 'browse') await phaseBrowse();
        if (p === 'gateway') await phaseGateway();
      } catch (e) {
        fail(p, e.message);
        if (p === 'build') break;
      }
      say(`${p}: ${((Date.now() - t) / 60000).toFixed(1)} min`);
    }
  } finally {
    if (!stopping) release();
  }
  if (!stopping) finish(null, summary.ok || (summary.differences.length === 0 && summary.failures.length === 0) ? 0 : 1);
}
