#!/usr/bin/env node
// The public gateway end to end (docs/rust-core.md, "Public mode"), on
// this machine only: urb-gateway -public test.example, as a fake moon of
// ~wep, behind a TLS proxy on loopback with a self-signed wildcard
// certificate, reading pages that ~wep and ~dep publish with lattice,
// checked with raw HTTPS requests and in headless Brave.
//
//   FLEET_WHO=<claim> node core/ci/public-gateway.mjs [--out DIR]
//
// It needs the fleet sets lattice-gate (~dep) and lattice-gate-peer
// (~wep) claimed by FLEET_WHO and their `lattice` snapshots (it resets
// both to them, and again at the end), hoon-test-kit's ship-cookie.sh
// (COOKIE), openssl, Brave (PUBLIC_GATEWAY_BROWSER) and urb-gateway built (cargo build
// --release -p urb-gateway; URB_GATEWAY). Without them it says what is
// missing and exits 2, a skip, never a pass.
//
// Nothing leaves the machine: the gateway is a fake ship (urbit -F), the
// proxy and the gateway listen on loopback, test.example is resolved by
// the requests themselves (servername and Host, as curl --resolve) and
// by Brave's --host-resolver-rules, never by DNS.
//
// The TLS proxy is a few lines of node:https standing in for Caddy: it
// terminates TLS and sets X-Forwarded-For to the client's address, as
// Caddy does for an untrusted client. Requests from 127.0.0.N reach the
// gateway as client 127.0.0.N, so each check that spends a client's
// rate limit has an address of its own.

import { spawn, spawnSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import https from 'node:https';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const FLEET = process.env.FLEET ?? `${process.env.HOME}/software/personal/hoon-development/fleet/fleet`;
const COOKIE = process.env.COOKIE ?? `${process.env.HOME}/software/personal/furum/scripts/hoon-test-kit/ship-cookie.sh`;
const RUST = process.env.URB_GATEWAY ?? join(root, 'core/target/release/urb-gateway');
const BROWSER = process.env.PUBLIC_GATEWAY_BROWSER || '/opt/brave.com/brave/brave';
const WHO = process.env.FLEET_WHO;
const SNAP = 'lattice';
const DOMAIN = 'test.example';
const TLS_PORT = 8443;
const GW = '127.0.0.1:18475';
const METRICS = '127.0.0.1:18476';
const SHIPS = { wep: { set: 'lattice-gate-peer', port: 8151 }, dep: { set: 'lattice-gate', port: 8150 } };

const argv = process.argv.slice(2);
let out = null;
if (argv[0] === '--out') out = argv[1];
out ??= mkdtempSync(join(tmpdir(), 'public-gateway-'));
mkdirSync(out, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));

// ------------------------------------------------------------ preflight

const missing = [];
if (!WHO) missing.push('FLEET_WHO');
for (const [p, what] of [[FLEET, 'fleet'], [COOKIE, 'ship-cookie.sh'], [RUST, 'urb-gateway (cargo build --release -p urb-gateway)'], [BROWSER, 'Brave']]) {
  if (!existsSync(p)) missing.push(what);
}
if (spawnSync('openssl', ['version']).status !== 0) missing.push('openssl');
if (WHO && existsSync(FLEET)) {
  const ls = spawnSync(FLEET, ['ls'], { encoding: 'utf8' }).stdout.split('\n');
  for (const { set } of Object.values(SHIPS)) {
    if (!ls.some(l => l.startsWith(`${set} `) && l.includes(`(${WHO})`))) missing.push(`a claim on ${set} by FLEET_WHO`);
  }
}
if (missing.length) {
  console.error(`public-gateway: SKIPPED, missing: ${missing.join(', ')}`);
  process.exit(2);
}

const results = [];
function check(name, ok, detail = '') {
  results.push([name, !!ok]);
  console.log(`${ok ? 'ok  ' : 'FAIL'}  ${name}${ok || !detail ? '' : `: ${String(detail).slice(0, 400)}`}`);
}

// ------------------------------------------------------------ the fleet

const cookies = {};
function reset() {
  for (const [ship, { set, port }] of Object.entries(SHIPS)) {
    const r = spawnSync(FLEET, ['--as', WHO, 'reset', set, SNAP], { encoding: 'utf8' });
    if (r.status !== 0) throw new Error(`fleet reset ${set}: ${r.stderr}`);
    const jar = join(out, `${ship}.jar`);
    let c;
    for (let i = 1; ; i++) {
      c = spawnSync(COOKIE, [`${process.env.HOME}/fleet/piers/${ship}`, `http://localhost:${port}`, jar], { encoding: 'utf8' });
      if (c.status === 0 || i >= 5) break;
      spawnSync('sleep', ['5']);
    }
    if (c.status !== 0) throw new Error(`ship-cookie ${ship}: ${c.stderr}`);
    const line = readFileSync(jar, 'utf8').split('\n').find(l => l.includes('urbauth-'));
    const f = line.split('\t');
    cookies[ship] = `${f[5]}=${f[6]}`;
  }
}

async function lattice(ship, route, body) {
  const r = await fetch(`http://localhost:${SHIPS[ship].port}/apps/lattice/${route}`, {
    method: 'POST', headers: { cookie: cookies[ship] }, body, signal: AbortSignal.timeout(60_000),
  });
  return `${r.status} ${await r.text()}`;
}
async function publish(ship, name, type, body) {
  const s = await lattice(ship, `page-save?name=${name}&type=${type}&new=1`, body);
  const p = await lattice(ship, `page-share?name=${name}&mode=urbit`);
  if (!s.startsWith('200') || !p.startsWith('200')) throw new Error(`publish ${ship} ${name}: ${s} / ${p}`);
}

// The pages. The hostile one runs its probes in its frame and writes
// what each found into its own body, for the browser checks to read.
const HOSTILE = `<!doctype html><html><head><title>hostile</title>
<base href="https://evil.example/"></head><body><p id="r">not run</p>
<script>
const r = {};
const t = (k, f) => { try { r[k] = String(f()); } catch (e) { r[k] = 'threw ' + e.name; } };
t('origin', () => self.origin);
t('cookie', () => document.cookie);
t('storage', () => localStorage.length);
t('parent', () => parent.document.title);
t('top', () => top.document.title);
t('topnav', () => { top.location.href = 'https://evil.example/'; return 'tried'; });
document.getElementById('r').textContent = JSON.stringify(r);
(async () => {
  for (const [k, u] of [['fetchShip', 'https://wep.${DOMAIN}:${TLS_PORT}/pubweb/note'], ['fetchApex', 'https://${DOMAIN}:${TLS_PORT}/']]) {
    try { const x = await fetch(u); r[k] = 'read ' + x.status; } catch (e) { r[k] = 'threw ' + e.name; }
  }
  document.getElementById('r').textContent = JSON.stringify(r);
})();
</script></body></html>`;
// ~dep's page tries to frame ~wep's pages and the apex
const FRAMER = `<!doctype html><html><body><p>framer</p>
<iframe id="a" src="https://wep.${DOMAIN}:${TLS_PORT}/pubweb/note"></iframe>
<iframe id="b" src="https://wep.${DOMAIN}:${TLS_PORT}/pubweb/hostile?frame"></iframe>
<iframe id="c" src="https://${DOMAIN}:${TLS_PORT}/"></iframe>
</body></html>`;

// ------------------------------------------------------------ TLS

function certificate() {
  const key = join(out, 'tls.key'), crt = join(out, 'tls.crt');
  const r = spawnSync('openssl', ['req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes',
    '-keyout', key, '-out', crt, '-days', '2', '-subj', `/CN=${DOMAIN}`,
    '-addext', `subjectAltName=DNS:${DOMAIN},DNS:*.${DOMAIN}`], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`openssl: ${r.stderr}`);
  return { key: readFileSync(key), cert: readFileSync(crt) };
}

function proxy(tls) {
  const [gh, gp] = GW.split(':');
  const s = https.createServer(tls, (req, res) => {
    const headers = { ...req.headers, 'x-forwarded-for': req.socket.remoteAddress.replace(/^::ffff:/, '') };
    const up = http.request({ host: gh, port: gp, method: req.method, path: req.url, headers }, u => {
      res.writeHead(u.statusCode, u.headers);
      u.pipe(res);
    });
    up.on('error', e => { res.writeHead(502); res.end(String(e)); });
    req.pipe(up);
  });
  return new Promise(r => s.listen(TLS_PORT, '127.0.0.1', () => r(s)));
}

// one request through the proxy, as curl --resolve: [status, headers, body]
// Brave is 127.0.0.1; these come from 127.0.0.2 to .6 in turn, so no
// check spends the browser's rate limit, and none spends its own
let turn = 0;
function ask(host, path, { from = `127.0.0.${2 + (turn++ % 5)}`, method = 'GET', headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.request({
      host: '127.0.0.1', port: TLS_PORT, servername: host.split(':')[0], method, path,
      localAddress: from, rejectUnauthorized: false, agent: false,
      headers: { host: `${host}:${TLS_PORT}`, ...headers },
    }, res => {
      let b = '';
      res.on('data', d => { b += d; });
      res.on('end', () => resolve([res.statusCode, res.headers, b]));
    });
    req.on('error', reject);
    req.setTimeout(90_000, () => req.destroy(new Error('timeout')));
    req.end();
  });
}

// ------------------------------------------------------------ the gateway

const SIS = 'marbinwansamlitsighidfidlissogdirwacsabwissib'.match(/.../g);
const DEX = 'zodnecbudwessevpersutletfulpensytdurwepserwylsun'.match(/.../g);
const pick = a => a[Math.floor(Math.random() * a.length)];
// a fake moon of ~wep no run has used, so ~wep has never heard it
const MOON = `~${pick(SIS)}${pick(DEX)}-${pick(SIS)}${pick(DEX)}-dozzod-dozwep`;

let gwLog = '';
async function startGateway(lists) {
  const state = join(out, 'state');
  const p = spawn(RUST, ['-public', DOMAIN, '-public-port', String(TLS_PORT), '-fake', MOON, '-state', state,
    '-listen', GW, '-metrics', METRICS, '-trust-proxy', '-abuse', 'abuse@test.example',
    '-start', 'urb://~wep/pubweb/start', '-optin', lists.optin, '-blocklist', lists.block]);
  p.stdout.on('data', d => { gwLog += d; });
  p.stderr.on('data', d => { gwLog += d; appendFileSync(join(out, 'gateway.log'), d); });
  const t = Date.now();
  while (!gwLog.includes('serving on port') && p.exitCode === null && Date.now() - t < 60_000) await sleep(100);
  if (!gwLog.includes('serving on port')) throw new Error(`the gateway did not start: ${gwLog}`);
  return p;
}

// ------------------------------------------------------------ Brave

function launch() {
  const profile = mkdtempSync(join(tmpdir(), 'public-gateway-profile-'));
  const b = spawn(BROWSER, ['--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
    `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--ignore-certificate-errors',
    `--host-resolver-rules=MAP ${DOMAIN} 127.0.0.1, MAP *.${DOMAIN} 127.0.0.1, MAP * ~NOTFOUND`, 'about:blank'],
    { stdio: ['ignore', 'ignore', 'pipe'] });
  process.on('exit', () => b.kill());
  const ws = new Promise((resolve, reject) => {
    let buf = '';
    b.stderr.on('data', c => { buf += c; const x = /DevTools listening on (ws:\/\/\S+)/.exec(buf); if (x) resolve(x[1]); });
    b.on('exit', c => reject(new Error(`browser exited ${c}\n${buf}`)));
    setTimeout(() => reject(new Error(`no devtools endpoint in 15 s\n${buf}`)), 15000);
  });
  return { b, ws };
}

async function browser() {
  const { b, ws: wsUrl } = launch();
  const ws = new WebSocket(await wsUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let seq = 0;
  const waiting = new Map();
  const logs = [];
  ws.onmessage = ev => {
    const m = JSON.parse(ev.data);
    if (m.method === 'Log.entryAdded') logs.push(m.params.entry.text);
    if (m.method === 'Target.attachedToTarget') {
      // a frame in a process of its own: its messages too, then let it run
      const s = m.params.sessionId;
      Promise.all([cdp('Log.enable', {}, s), cdp('Runtime.enable', {}, s),
        cdp('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }, s)])
        .catch(() => {}).finally(() => cdp('Runtime.runIfWaitingForDebugger', {}, s).catch(() => {}));
    }
    if (m.method === 'Runtime.consoleAPICalled') logs.push(m.params.args.map(a => a.value ?? '').join(' '));
    if (!m.id || !waiting.has(m.id)) return;
    const { r, j } = waiting.get(m.id);
    waiting.delete(m.id);
    m.error ? j(new Error(m.error.message)) : r(m.result);
  };
  const cdp = (method, params = {}, sessionId) => new Promise((r, j) => {
    const id = ++seq;
    waiting.set(id, { r, j });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
    setTimeout(() => { if (waiting.has(id)) { waiting.delete(id); j(new Error(`${method} timed out`)); } }, 120000);
  });
  const evaluate = async (expression, sessionId, contextId) => {
    const r = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, ...(contextId ? { contextId } : {}) }, sessionId);
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  await cdp('Target.setDiscoverTargets', { discover: true });
  const { targetId } = await cdp('Target.createTarget', { url: 'about:blank' });
  const { sessionId: p } = await cdp('Target.attachToTarget', { targetId, flatten: true });
  await cdp('Page.enable', {}, p);
  await cdp('Runtime.enable', {}, p);
  await cdp('Log.enable', {}, p);
  await cdp('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }, p);
  async function open(url, ms = 3000) {
    logs.length = 0;
    await cdp('Page.navigate', { url }, p);
    for (let i = 0; i < 100; i++) {
      if (await evaluate('document.readyState', p).catch(() => '') === 'complete') break;
      await sleep(200);
    }
    await sleep(ms);
  }
  // a frame's text: an out-of-process frame through its own target, or
  // one in the page's process through an isolated world in it
  async function frameText(match) {
    const { targetInfos } = await cdp('Target.getTargets');
    const t = targetInfos.find(x => x.type === 'iframe' && match(x.url));
    if (t) {
      const { sessionId } = await cdp('Target.attachToTarget', { targetId: t.targetId, flatten: true });
      try { return await evaluate('document.body ? document.body.innerText : ""', sessionId); }
      finally { await cdp('Target.detachFromTarget', { sessionId }).catch(() => {}); }
    }
    const { frameTree } = await cdp('Page.getFrameTree', {}, p);
    const all = [];
    const walk = f => { all.push(f.frame); (f.childFrames || []).forEach(walk); };
    walk(frameTree);
    const f = all.find(x => match(x.url));
    if (!f) return null;
    const { executionContextId } = await cdp('Page.createIsolatedWorld', { frameId: f.id }, p);
    return evaluate('document.body ? document.body.innerText : ""', p, executionContextId);
  }
  async function frames() {
    const { frameTree } = await cdp('Page.getFrameTree', {}, p);
    const all = [];
    const walk = f => { all.push(f.frame); (f.childFrames || []).forEach(walk); };
    walk(frameTree);
    const { targetInfos } = await cdp('Target.getTargets');
    return { tree: all.map(f => f.url), targets: targetInfos.filter(t => t.type === 'iframe').map(t => t.url) };
  }
  return { open, evaluate: e => evaluate(e, p), frameText, frames, logs, close: () => b.kill() };
}

// ------------------------------------------------------------ the run

const lists = { optin: join(out, 'optin'), block: join(out, 'blocklist') };
writeFileSync(lists.optin, '# test publishers\n~wep\n~dep\n');
writeFileSync(lists.block, '# nothing yet\n');

console.log(`public-gateway: ${out}, as ${MOON}`);
reset();
const W = `wep.${DOMAIN}`, D = `dep.${DOMAIN}`;
await publish('wep', 'pubweb/note', 'md', '# A note\n\nPublished for the web. <script>alert(1)</script>\n\n[to dep](urb://~dep/pubweb/framer)');
await publish('wep', 'pubweb/gem', 'gmi', '# Gemtext\n=> urb://~wep/pubweb/note the note\nplain line');
await publish('wep', 'pubweb/hostile', 'html', HOSTILE);
await publish('wep', 'pubweb/start', 'gmi', '=> urb://~wep/pubweb/note A note on ~wep\n=> urb://~dep/pubweb/framer A page on ~dep');
await publish('dep', 'pubweb/framer', 'html', FRAMER);
await publish('dep', 'pubweb/blocked', 'md', '# soon blocked');
await sleep(3000);

const tls = certificate();
const px = await proxy(tls);
const gw = await startGateway(lists);
let failed = false;
try {
  // -------- reads, through the proxy
  let [st, h, b] = await ask(W, '/pubweb/note');
  for (let i = 0; st !== 200 && i < 5; i++) { await sleep(3000); [st, h, b] = await ask(W, '/pubweb/note'); }
  check('a Markdown page reads', st === 200 && b.includes('<h1>A note</h1>'), `${st} ${b}`);
  check('its link to another ship points at that ship\'s host', b.includes(`href="https://dep.${DOMAIN}:${TLS_PORT}/pubweb/framer"`), b);
  check('the page\'s script is text', !/<script/i.test(b) && b.includes('&lt;script&gt;'), b);
  check('the chrome shows the actions disabled with their reasons',
    ['bookmark', 'comment', 'follow'].every(a => b.includes(`disabled title="Connect a ship to ${a}"`)), b);
  check('the chrome has the abuse contact and Open in lattice',
    b.includes('Report abuse: abuse@test.example') && b.includes('href="urb://~wep/pubweb/note">Open in lattice'), b);
  check('the chrome\'s policy allows no script', /^default-src 'none'; style-src 'unsafe-inline';/.test(h['content-security-policy']) && !/script-src/.test(h['content-security-policy']), h['content-security-policy']);
  for (const [k, v] of [['strict-transport-security', 'max-age=31536000; includeSubDomains'], ['x-content-type-options', 'nosniff'],
    ['referrer-policy', 'no-referrer'], ['cross-origin-opener-policy', 'same-origin'], ['x-robots-tag', 'noindex, nofollow'], ['cache-control', 'public, max-age=60']]) {
    check(`header ${k}`, h[k] === v, h[k]);
  }
  check('no cookie is set', !h['set-cookie']);
  [st, , b] = await ask(W, '/pubweb/gem');
  check('a gemtext page reads', st === 200 && b.includes('<h1>Gemtext</h1>') && b.includes(`href="https://wep.${DOMAIN}:${TLS_PORT}/pubweb/note"`), `${st} ${b}`);
  [st, h, b] = await ask(W, '/pubweb/hostile');
  check('an HTML page is the interstitial and a sandboxed frame', st === 200 && b.includes('This page is from ~wep, not from test.example.')
    && b.includes('sandbox="allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation"') && b.includes('src="?frame"')
    && !b.includes('allow-same-origin') && !/<script/i.test(b), b);
  [st, h, b] = await ask(W, '/pubweb/hostile?frame', { headers: { 'sec-fetch-dest': 'iframe', 'sec-fetch-site': 'same-origin' } });
  check('the frame holds the publisher\'s HTML under the sandbox policy', st === 200 && b.includes('<script>')
    && h['content-security-policy'] === "sandbox allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation; frame-ancestors 'self'", `${st} ${h['content-security-policy']}`);
  [st] = await ask(W, '/pubweb/hostile?frame', { headers: { 'sec-fetch-dest': 'document' } });
  check('the frame as a top-level page is 403', st === 403, st);
  [st, h, b] = await ask(W, '/robots.txt');
  check('robots.txt disallows everything', st === 200 && b === 'User-agent: *\nDisallow: /\n', b);
  [st, , b] = await ask(DOMAIN, '/');
  check('the home page has the form and the start list read from ~wep', st === 200 && b.includes('action="/go"')
    && b.includes(`<a href="https://wep.${DOMAIN}:${TLS_PORT}/pubweb/note">A note on ~wep</a>`), b);
  [st, h] = await ask(DOMAIN, `/go?url=${encodeURIComponent('~dep/pubweb/framer')}`);
  check('/go sends ~ship/page to its host', st === 302 && h.location === `https://dep.${DOMAIN}:${TLS_PORT}/pubweb/framer`, `${st} ${h.location}`);
  [st, , b] = await ask(W, '/pubweb/missing');
  check('a missing page has the gateway\'s words', st === 404 && b.includes('urb://~wep/pubweb/missing is not published.'), `${st} ${b}`);
  [st, , b] = await ask(W, '/forum/general');
  check('a forum board is 501, coming', st === 501 && b.includes('Reading boards on the web is coming.'), st);
  [st, , b] = await ask(`zod.${DOMAIN}`, '/');
  check('a ship that has not opted in is 403, and not read', st === 403 && b.includes('~zod has not chosen'), `${st} ${b}`);
  for (const host of ['evil.example', 'wep.urb.localhost', 'localhost', 'a.wep.test.example', 'www.test.example']) {
    [st] = await ask(host, '/');
    check(`Host ${host} is 421`, st === 421, st);
  }
  [st] = await ask(W, '/pubweb/note', { method: 'POST' });
  check('a POST is 405', st === 405, st);

  // -------- the blocklist, edited while it runs
  [st] = await ask(D, '/pubweb/blocked', { from: '127.0.0.7' });
  check('a page reads before it is blocked', st === 200, st);
  writeFileSync(lists.block, '~dep/pubweb/blocked\n~bus\n');
  await sleep(1100);
  [st, , b] = await ask(D, '/pubweb/blocked', { from: '127.0.0.7' });
  check('after the edit it is 451 with the abuse contact, no restart', st === 451 && b.includes('abuse@test.example'), `${st} ${b}`);
  [st] = await ask(D, '/pubweb/framer', { from: '127.0.0.7' });
  check('the rest of the ship still reads', st === 200, st);

  // -------- the rate limit, from a client of its own
  const codes = [];
  for (let i = 0; i < 25; i++) codes.push((await ask(W, '/pubweb/note', { from: '127.0.0.9' }))[0]);
  const first429 = codes.indexOf(429);
  check('a client is 429 after its burst of 20', codes.slice(0, 20).every(c => c === 200) && first429 >= 20, codes.join(','));
  [st, h] = await ask(W, '/pubweb/note', { from: '127.0.0.9' });
  check('the 429 says when to come back', st === 429 && Number(h['retry-after']) >= 1, `${st} ${h['retry-after']}`);
  [st] = await ask(W, '/pubweb/note', { from: '127.0.0.10' });
  check('another client is not limited', st === 200, st);
  // X-Forwarded-For from the client is replaced by the proxy's
  [st] = await ask(W, '/pubweb/note', { from: '127.0.0.9', headers: { 'x-forwarded-for': '192.0.2.50' } });
  check('a client can\'t pose as another with X-Forwarded-For', st === 429, st);

  // -------- the browser
  const br = await browser();
  try {
    await br.open(`https://${W}:${TLS_PORT}/pubweb/note`, 1000);
    check('Brave shows the note', (await br.evaluate('document.body.innerText')).includes('A note'));
    check('Brave: the buttons are disabled', await br.evaluate(`[...document.querySelectorAll('ul.acts button')].length === 3 && [...document.querySelectorAll('ul.acts button')].every(b => b.disabled)`));
    check('Brave: no script element in the chrome', await br.evaluate(`document.querySelectorAll('script').length === 0`));
    await br.open(`https://${W}:${TLS_PORT}/pubweb/hostile`, 4000);
    check('Brave: the chrome is untouched by the frame\'s script', await br.evaluate(`!document.body.innerText.includes('owned') && document.title === 'urb://~wep/pubweb/hostile' && location.host === '${W}:${TLS_PORT}'`));
    const txt = await br.frameText(u => u.includes('/pubweb/hostile?frame'));
    let r = {};
    try { r = JSON.parse(txt); } catch { /* checked below */ }
    check('Brave: the frame\'s script ran, in the frame', r.origin === 'null', txt);
    check('Brave: the frame cannot read a cookie or storage', /^threw/.test(r.cookie) && /^threw/.test(r.storage), txt);
    check('Brave: the frame cannot script its ship\'s chrome', /^threw/.test(r.parent) && /^threw/.test(r.top), txt);
    check('Brave: the frame cannot read its ship\'s pages or the apex', /^threw/.test(r.fetchShip) && /^threw/.test(r.fetchApex), txt);
    check('Brave: the frame cannot navigate the tab without a click', await br.evaluate(`location.host === '${W}:${TLS_PORT}'`));
    await br.open(`https://${D}:${TLS_PORT}/pubweb/framer`, 4000);
    const fr = await br.frames();
    writeFileSync(join(out, 'framer-frames.json'), JSON.stringify({ ...fr, logs: br.logs }, null, 2));
    const framerTxt = await br.frameText(u => u.includes('/pubweb/framer?frame'));
    check('Brave: ~dep\'s page renders in its frame', (framerTxt ?? '').includes('framer'), framerTxt);
    const refused = br.logs.filter(l => /frame-ancestors/.test(l));
    const errored = [...fr.tree, ...fr.targets].filter(u => u.startsWith('chrome-error:'));
    check('Brave: ~dep\'s page cannot frame ~wep\'s pages or the apex (frame-ancestors)', refused.length >= 3 || errored.length >= 3,
      JSON.stringify({ logs: br.logs, ...fr }));
    const leaked = [];
    for (const m of [u => u.includes('wep.') && u.includes('/pubweb/note'), u => u.includes('/pubweb/hostile?frame'), u => u.startsWith(`https://${DOMAIN}`)]) {
      const t = await br.frameText(m).catch(() => null);
      // a blocked frame holds the browser's own error page, which names
      // the host; anything of the page itself would be a leak
      if (t && /A note|Published for the web|origin|not run|Start here|abuse@/.test(t)) leaked.push(t.slice(0, 80));
    }
    check('Brave: nothing of ~wep or the apex shows inside ~dep\'s page', leaked.length === 0, leaked.join(' | '));
    await br.open(`https://${DOMAIN}:${TLS_PORT}/`, 500);
    check('Brave: the home page shows the start list', (await br.evaluate('document.body.innerText')).includes('A note on ~wep'));
  } finally {
    br.close();
  }

  // -------- the counters, on their own loopback listener
  const m = await (await fetch(`http://${METRICS}/metrics`)).text();
  check('the counters count reads, cache hits, 429s and 451s', /urb_gateway_reads_total [1-9]/.test(m) && /urb_gateway_cache_hits_total [1-9]/.test(m)
    && /urb_gateway_limited_total [1-9]/.test(m) && /urb_gateway_blocked_total [1-9]/.test(m), m);
} catch (e) {
  failed = true;
  console.log(`FAIL  the run stopped: ${e.stack}`);
} finally {
  gw.kill('SIGINT');
  await sleep(1500);
  px.close();
}

// -------- the log: status, duration and ship; never a path or an IP
const lines = gwLog.split('\n').filter(Boolean).map(l => l.replace(/^\d{4}\/\d\d\/\d\d \d\d:\d\d:\d\d /, ''));
const after = lines.slice(lines.findIndex(l => l.startsWith('serving on port')) + 1);
const reqLines = after.filter(l => /^\d{3} /.test(l));
const odd = after.filter(l => !/^\d{3} (~[a-z-]+|-) \d+ms$/.test(l) && !/^list (optin|blocklist): /.test(l));
check('the log has one line per request', reqLines.length > 40, reqLines.length);
check('each request line is status, ship and duration only', odd.length === 0, odd.join(' | '));
const bad = lines.filter(l => /pubweb|127\.0\.0\.|192\.0\.2|evil\.example|\?frame|robots/.test(l));
check('the log holds no path, query or address', bad.length === 0, bad.join(' | '));
writeFileSync(join(out, 'gateway-lines.txt'), lines.join('\n'));

// leave the sets as their snapshots
try { reset(); } catch (e) { console.log(`note: the final reset failed: ${e.message}`); }
const pass = results.filter(r => r[1]).length;
console.log(`\npublic-gateway: ${pass} of ${results.length} passed${failed ? ', and the run stopped early' : ''} (${out})`);
process.exit(pass === results.length && !failed ? 0 : 1);
