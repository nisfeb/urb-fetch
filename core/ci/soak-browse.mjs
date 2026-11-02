#!/usr/bin/env node
// Gate 5's "normal use" pass (docs/rust-core.md, "Gate 5: automated
// soak"): a throwaway headless Brave loads one copy of the extension
// and uses it for about ten minutes against the fake fleet, the way a
// person would, and records every page it showed and every error.
// core/ci/soak.mjs runs it once on the Rust lib/ and once on the JS
// lib/, each after a reset, and diffs the two records.
//
//   URB_FETCH=<go binary> READER_JAR=<~hus cookie jar> \
//     node core/ci/soak-browse.mjs <extension dir> <out dir>
//
// The steps, in a fixed order so the two records line up: as a comet,
// read ~hus's index, fleet-check, big and each marks/ page, a missing
// page, a lattice forum page and ~rup (which runs no lattice), and
// follow the urb:// links those pages hold; then as a fake moon of ~rup
// publish a page and an edit of it, each read back by Lattice on ~hus;
// then as a fake moon of ~hus read the furum board ~rup/m1c and a post
// on it, and comment on that post. DWELL seconds (default 25) of
// reading time pass between steps. SOAK_BROWSER names the binary; the
// profile is a new temporary folder, removed at the end, never anyone's
// own.
//
// It writes <out>/steps.json ([{n, name, url, ms, status, body, frame,
// errors}]) and <out>/NN-name.txt per step. Exit 0 when it ran to the
// end (pages that failed are recorded, not fatal), 1 when it could not.

import { spawn } from 'node:child_process';
import { appendFileSync, cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const [EXT, OUT] = process.argv.slice(2);
const { URB_FETCH, READER_JAR } = process.env;
const BROWSER = process.env.SOAK_BROWSER || '/opt/brave.com/brave/brave';
const DWELL = Number(process.env.DWELL ?? 25) * 1000;
const READER = process.env.READER || 'http://localhost:8134';
const BOARD = process.env.BOARD || '~rup/m1c';
if (!EXT || !OUT || !URB_FETCH || !READER_JAR) {
  console.error('usage: URB_FETCH=... READER_JAR=... soak-browse.mjs <extension dir> <out dir>');
  process.exit(2);
}
mkdirSync(OUT, { recursive: true });
const { readerPath } = await import(join(EXT, 'lib/gemtext.js'));
const { fakeKeyfile, shipName } = await import(join(EXT, 'lib/mesa.mjs'));
const { URB_ORIGINS } = await import(join(EXT, 'lib/urb.js'));
const wasm = readFileSync(join(EXT, 'lib/mesa.wasm'));
const sleep = ms => new Promise(r => setTimeout(r, ms));

// ---------------------------------------------------------------- relay

const relayState = mkdtempSync(join(tmpdir(), 'soak-relay-'));
const port = 30000 + Math.floor(Math.random() * 20000);
const relay = spawn(URB_FETCH, ['-relay', `127.0.0.1:${port}`, '-state', relayState, '-relay-fake'], { stdio: ['ignore', 'ignore', 'pipe'] });
{
  let log = '';
  relay.stderr.on('data', c => { log += c; });
  await new Promise((resolve, reject) => {
    const t = setInterval(() => { if (log.includes('relaying at')) { clearInterval(t); resolve(); } }, 100);
    relay.on('exit', c => reject(new Error(`the relay exited ${c}\n${log}`)));
    setTimeout(() => reject(new Error(`the relay did not start\n${log}`)), 10000);
  });
}
const RELAY = `ws://127.0.0.1:${port}/`;
process.on('exit', () => relay.kill());

// ------------------------------------------- stage, grant, launch (as urb-check.js)

const stage = mkdtempSync(join(tmpdir(), 'soak-ext-'));
const profile = mkdtempSync(join(tmpdir(), 'soak-profile-'));
cpSync(EXT, stage, { recursive: true, filter: src => !/^\/(\.git|node_modules|test|scripts|docs)(\/|$)/.test(src.slice(EXT.length)) });
function launch() {
  const browser = spawn(BROWSER, [
    '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
    `--user-data-dir=${profile}`, '--remote-debugging-port=0',
    `--load-extension=${stage}`, `--disable-extensions-except=${stage}`, 'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  process.on('exit', () => browser.kill());
  const wsUrl = new Promise((resolve, reject) => {
    let buf = '';
    browser.stderr.on('data', c => { buf += c; const x = /DevTools listening on (ws:\/\/\S+)/.exec(buf); if (x) resolve(x[1]); });
    browser.on('exit', c => reject(new Error(`browser exited ${c}\n${buf}`)));
    setTimeout(() => reject(new Error(`no devtools endpoint in 15 s\n${buf}`)), 15000);
  });
  return { browser, wsUrl };
}
{
  const first = launch();
  await first.wsUrl;
  await sleep(2500);
  first.browser.kill();
  await new Promise(r => first.browser.once('exit', r));
  const pf = join(profile, 'Default', 'Preferences');
  const prefs = JSON.parse(readFileSync(pf, 'utf8'));
  const settings = prefs.extensions.settings;
  const e = settings[Object.keys(settings).find(k => settings[k].path === stage)];
  for (const k of ['active_permissions', 'granted_permissions', 'runtime_granted_permissions']) {
    e[k] = { ...(e[k] || {}), api: (e[k] && e[k].api) || [], explicit_host: URB_ORIGINS, manifest_permissions: [], scriptable_host: [] };
  }
  delete e.withholding_permissions;
  writeFileSync(pf, JSON.stringify(prefs));
}
const { browser, wsUrl } = launch();
const ws = new WebSocket(await wsUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });

let seq = 0;
const waiting = new Map();
let errors = [];   // since the step began
ws.onmessage = ev => {
  const msg = JSON.parse(ev.data);
  if (msg.method === 'Runtime.exceptionThrown') {
    const d = msg.params.exceptionDetails;
    errors.push(`exception: ${d.exception?.description || d.text}`);
  }
  if (msg.method === 'Runtime.consoleAPICalled' && (msg.params.type === 'error' || msg.params.type === 'assert')) {
    errors.push(`console.${msg.params.type}: ${msg.params.args.map(a => a.value ?? a.description ?? '').join(' ')}`);
  }
  if (msg.method === 'Log.entryAdded' && msg.params.entry.level === 'error') {
    errors.push(`log: ${msg.params.entry.text}${msg.params.entry.url ? ` (${msg.params.entry.url})` : ''}`);
  }
  if (!msg.id || !waiting.has(msg.id)) return;
  const { r, j } = waiting.get(msg.id);
  waiting.delete(msg.id);
  msg.error ? j(new Error(msg.error.message)) : r(msg.result);
};
const cdp = (method, params = {}, sessionId) => new Promise((r, j) => {
  const id = ++seq;
  waiting.set(id, { r, j });
  ws.send(JSON.stringify({ id, method, params, sessionId }));
  setTimeout(() => { if (waiting.has(id)) { waiting.delete(id); j(new Error(`${method} timed out`)); } }, 330000);
});
async function evaluate(expression, sessionId) {
  const r = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId);
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
}
const watch = async s => { await cdp('Runtime.enable', {}, s); await cdp('Log.enable', {}, s).catch(() => {}); };

// the worker, found again each time: the browser stops it when idle, as
// it would for anyone
async function worker() {
  for (let i = 0; i < 50; i++) {
    const { targetInfos } = await cdp('Target.getTargets');
    const t = targetInfos.find(x => x.type === 'service_worker' && x.url.endsWith('/background.js'));
    if (t) {
      const { sessionId } = await cdp('Target.attachToTarget', { targetId: t.targetId, flatten: true });
      await watch(sessionId);
      for (let j = 0; j < 50; j++) {
        if (await evaluate('typeof nisfeb', sessionId).catch(() => '') === 'object') return { sessionId, url: t.url };
        await sleep(100);
      }
    }
    await sleep(200);
  }
  throw new Error('the extension worker never appeared');
}
const handle = async msg => {
  const w = await worker();
  try { return await evaluate(`nisfeb.handle(${JSON.stringify(msg)})`, w.sessionId); }
  finally { await cdp('Target.detachFromTarget', { sessionId: w.sessionId }).catch(() => {}); }
};

// one tab, used throughout, as a person would
const first = await worker();
const BASE = `chrome-extension://${new URL(first.url).host}/`;
await evaluate(`chrome.storage.local.set(${JSON.stringify({ urbReader: 'browser', urbRelay: RELAY, urbFake: true })})`, first.sessionId);
await cdp('Target.detachFromTarget', { sessionId: first.sessionId });
const { targetId: tab } = await cdp('Target.createTarget', { url: 'about:blank' });
const { sessionId: p } = await cdp('Target.attachToTarget', { targetId: tab, flatten: true });
await watch(p);
await cdp('Page.enable', {}, p);

const BUSY = /^(Reading|Still waiting|Posting|Commenting|Voting|Publishing$|Saving|.* took it in; waiting)/;
const statusOf = () => evaluate(`(document.getElementById('status') || {}).textContent || ''`, p).catch(() => '');
// waits until the page's status line says it is done, and holds still
async function settled(ms = 400000) {
  let last = null, same = 0;
  for (const end = Date.now() + ms; Date.now() < end;) {
    const s = await statusOf();
    if (s && !BUSY.test(s) && s === last) { if (++same >= 2) return s; } else same = 0;
    last = s;
    await sleep(500);
  }
  return `${last} (still busy after ${ms / 1000} s)`;
}
// the sandboxed frame's text, when the page shows HTML
async function frameText() {
  const { targetInfos } = await cdp('Target.getTargets');
  const t = targetInfos.find(x => x.type === 'iframe' && x.url.endsWith('/frame.html'));
  if (!t) return null;
  const { sessionId } = await cdp('Target.attachToTarget', { targetId: t.targetId, flatten: true });
  try { return await evaluate('document.body.innerText', sessionId); }
  catch (e) { return `unreadable: ${e.message}`; }
  finally { await cdp('Target.detachFromTarget', { sessionId }).catch(() => {}); }
}

const steps = [];
async function step(name, url, act) {
  const n = steps.length + 1;
  errors = [];
  const t = Date.now();
  let status, body, frame = null;
  try {
    if (url) await cdp('Page.navigate', { url: BASE + url }, p);
    await sleep(300);
    if (act) await act();
    status = await settled();
    body = await evaluate('document.body.innerText', p).catch(e => `unreadable: ${e.message}`);
    frame = await frameText();
  } catch (e) {
    status = `step failed: ${e.message}`;
    body = body ?? '';
  }
  await sleep(500);
  const rec = { n, name, url, ms: Date.now() - t, status, body, frame, errors };
  steps.push(rec);
  writeFileSync(join(OUT, `${String(n).padStart(2, '0')}-${name}.txt`),
    `${url ?? ''}\nstatus: ${status}\n\n${body}\n${frame === null ? '' : `\n--- frame ---\n${frame}\n`}${errors.length ? `\n--- errors ---\n${errors.join('\n')}\n` : ''}`);
  console.log(`${String(n).padStart(2, '0')} ${name}: ${status.slice(0, 100)} (${(rec.ms / 1000).toFixed(1)} s, ${errors.length} errors)`);
  await sleep(DWELL);
  return rec;
}

// adds a line to the last step's record, for what was checked outside
// the browser
function note(line) {
  const rec = steps.at(-1);
  rec.body += `\n${line}`;
  appendFileSync(join(OUT, `${String(rec.n).padStart(2, '0')}-${rec.name}.txt`), `${line}\n`);
  console.log(`   ${line}`);
}

const cookie = readFileSync(READER_JAR, 'utf8').split('\n')
  .map(l => l.replace(/^#HttpOnly_/, '').split('\t')).find(f => f.length === 7 && f[5].startsWith('urbauth-'));
// Lattice on ~hus reads a page, again until it has the body wanted or
// 90 s pass (it may hold an older revision for a while)
async function latticeReads(url, want) {
  let got;
  for (const end = Date.now() + 90000; ;) {
    const r = await fetch(`${READER}/apps/lattice/fetch?url=${encodeURIComponent(url)}`, {
      headers: { cookie: `${cookie[5]}=${cookie[6]}` }, signal: AbortSignal.timeout(90000),
    }).catch(e => ({ status: 0, text: async () => e.message }));
    const text = await r.text();
    try { got = { status: r.status, body: JSON.parse(text).body }; } catch { got = { status: r.status, body: text }; }
    if (got.body === want || Date.now() > end) return got;
    await sleep(5000);
  }
}

let ranToEnd = false;
try {
  // ---------------------------------------------- reading, as a comet
  const pages = ['~hus/', '~hus/fleet-check', '~hus/big', '~hus/marks/md', '~hus/marks/html', '~hus/marks/text',
    '~hus/marks/gmi', '~hus/marks/css', '~hus/no-such-page', '~hus/forum/m1btest', '~rup/page'];
  for (const pg of pages) {
    const url = `urb://${pg}`;
    const name = `read-${pg.replace(/[^a-z0-9]+/g, '-').replace(/-$/, '')}`;
    await step(name, readerPath(url));
    // following a link: the page's first link to another page, clicked
    // as a person would, from the page as it was shown
    const links = await evaluate(`[...document.querySelectorAll('a[href]')].filter(a => a.textContent.trim() && !/^(https?:|mailto:|#)/.test(a.getAttribute('href'))).map(a => a.getAttribute('href'))`, p).catch(() => []);
    if (links.length) {
      await step(`follow-${name.slice(5)}`, readerPath(url), async () => {
        await settled();
        await evaluate(`[...document.querySelectorAll('a[href]')].find(a => a.getAttribute('href') === ${JSON.stringify(links[0])}).click()`, p);
        await sleep(1000);
      });
      note(`followed ${links[0]}, now at ${await evaluate('location.href', p).then(h => h.replace(BASE, '')).catch(e => e.message)}`);
    }
  }

  // ---------------------------------------- publishing, as a moon of ~rup
  const names = await Promise.all(Array.from({ length: 256 }, (_, g) => shipName(wasm, BigInt(g))));
  const k = BigInt(Math.floor(Date.now() / 1000) % 0xfffffff0) + 1n;
  const pubMoon = await shipName(wasm, (k << 32n) | BigInt(names.indexOf('~rup')));
  const furMoon = await shipName(wasm, ((k + 1n) << 32n) | BigInt(names.indexOf('~hus')));
  const add = async m => handle({ kind: 'urbMoonAdd', keyfile: await fakeKeyfile(wasm, m), passphrase: 'a long passphrase for the soak' });
  const added = await add(pubMoon);
  console.log(`   publishing as ${pubMoon}: ${added.ok ? 'added' : JSON.stringify(added)}`);
  await step('publish-open', 'publish.html');
  const v1 = '# Soak notes\n\nA page published from publish.html during the soak.\n\n=> urb://~hus/fleet-check the fleet check\n';
  const v2 = `${v1}\nAn edit: a second paragraph.\n`;
  const put = body => async () => {
    await evaluate(`document.getElementById('name').value = 'notes/soak'; document.getElementById('body').value = ${JSON.stringify(body)}; document.getElementById('put').requestSubmit()`, p);
    for (let i = 0; i < 120; i++) {
      const t = await evaluate('document.body.innerText', p).catch(() => '');
      if (t.includes(`revision ${body === v1 ? 1 : 2}`)) break;
      await sleep(500);
    }
  };
  await step('publish-page', null, put(v1));
  const r1 = await latticeReads(`urb://${pubMoon}/notes/soak`, v1);
  note(`~hus reads revision 1: ${r1.status} ${r1.body === v1 ? 'as published' : JSON.stringify(String(r1.body).slice(0, 300))}`);
  await step('publish-edit', null, put(v2));
  const r2 = await latticeReads(`urb://${pubMoon}/notes/soak`, v2);
  note(`~hus reads revision 2: ${r2.status} ${r2.body === v2 ? 'as edited' : JSON.stringify(String(r2.body).slice(0, 300))}`);
  await step('read-own-page', readerPath(`urb://${pubMoon}/notes/soak`));
  const gone = await handle({ kind: 'urbMoonRemove' });
  console.log(`   moon removed: ${gone.ok}`);

  // ------------------------------------------- furum, as a moon of ~hus
  const [host, board] = BOARD.split('/');
  const joined = await add(furMoon);
  console.log(`   furum as ${furMoon}: ${joined.ok ? 'added' : JSON.stringify(joined)}`);
  await step('furum-board', `furum.html?b=${host}/${board}`);
  const posts = await handle({ kind: 'furum', op: 'posts', host, board });
  const id = posts.ok && posts.posts.length ? Math.min(...posts.posts.map(x => Number(x.id))) : null;
  if (id === null) console.log(`   no post to comment on: ${JSON.stringify(posts).slice(0, 200)}`);
  else {
    await step('furum-thread', `furum.html?b=${host}/${board}&p=${id}`);
    await step('furum-comment', null, async () => {
      await evaluate(`document.getElementById('cbody').value = 'A comment from the soak, in a headless Brave.'; document.getElementById('commentform').requestSubmit()`, p);
      await sleep(1000);
    });
    await step('furum-board-again', `furum.html?b=${host}/${board}`);
  }
  await step('read-again-fleet-check', readerPath('urb://~hus/fleet-check'));
  ranToEnd = true;
} catch (e) {
  console.error(`soak-browse: stopped: ${e.stack || e.message}`);
  steps.push({ n: steps.length + 1, name: 'stopped', url: null, ms: 0, status: `stopped: ${e.message}`, body: '', frame: null, errors });
} finally {
  writeFileSync(join(OUT, 'steps.json'), JSON.stringify(steps, null, 1));
  await sleep(1000);
  browser.kill();
  relay.kill();
  await new Promise(r => browser.once('exit', r));
  for (const d of [stage, profile, relayState]) rmSync(d, { recursive: true, force: true });
}
process.exit(ranToEnd ? 0 : 1);
