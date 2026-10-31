// Records docs/rust-core.md's traces 1 to 9 against the fleet's furum set
// (~rup with furum release 7, ~hus with lattice release 81), restored to
// its m1c-traces snapshot (traces/fixtures.sh says what that holds):
//
//   node record.mjs [--out DIR] [--hus JAR] [--rup JAR] [N ...]
//
// It writes DIR/<n>-<name>.jsonl (default ../traces), for every trace or
// the ones named. JAR is a cookie jar for the ship's HTTP (made by
// hoon-test-kit's ship-cookie.sh): ~hus's for trace 5 (lattice reads,
// keeps and drops our page), 6 (a page listed under a stale id) and 9 (the salt
// rotated); ~rup's is not needed. Each trace runs as its own fake moon
// of ~hus, so no ship has heard of it before, except trace 1's comet,
// mined afresh. MESA_TRACE must be unset.

import dgram from 'node:dgram';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { DESK_BASE, fakeKeyfile } from './mesa.mjs';
import { recorder, toLines } from './trace.mjs';

const { values: opt, positionals } = parseArgs({
  allowPositionals: true,
  options: { out: { type: 'string', default: new URL('../traces', import.meta.url).pathname },
             hus: { type: 'string', default: '/tmp/m1c/hus.jar' }, rup: { type: 'string' } },
});
const wasm = readFileSync(new URL('build/mesa.wasm', import.meta.url));
const HUS = 'http://localhost:8134/apps/lattice';
const log = (...a) => console.error(new Date().toISOString().slice(11, 19), ...a);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const MOON = n => `~natdul-mol${['', 'nec', 'bud', 'wes', 'sev', 'per', 'sut', 'let', 'ful', 'pen'][n]}-dozzod-dozhus`;

// UDP on the fake network: galaxy g at 127.0.0.1:31337+g. Bound before
// the engine opens, so start is synchronous, as trace.mjs needs; one
// socket for a whole trace, so a second run keeps the first's port.
async function udp() {
  const sock = dgram.createSocket('udp4');
  await new Promise((ok, no) => { sock.once('error', no); sock.bind(0, '127.0.0.1', ok); });
  let heard = null;
  const ipNum = a => a.split('.').reduce((v, o) => ((v << 8) | Number(o)) >>> 0, 0);
  sock.on('message', (msg, from) => heard?.(ipNum(from.address), from.port, new Uint8Array(msg)));
  return {
    start(ship, f) { heard = f; },
    send(lane, bytes) { sock.send(bytes, lane.name ? 31337 + lane.galaxy : lane.port, '127.0.0.1'); },
    close() { heard = null; },
    end: () => sock.close(),
  };
}

// ~hus's lattice over HTTP, as its owner
function ship(base, jar) {
  const cookie = () => readFileSync(jar, 'utf8').split('\n').map(l => l.split('\t'))
    .filter(p => p.length === 7 && p[5].startsWith('urbauth-')).map(p => `${p[5]}=${p[6]}`)[0];
  return async (method, path, body) => {
    const r = await fetch(base + path, { method, body, headers: { cookie: cookie() } });
    const text = await r.text();
    log(`  ${method} ${path.slice(0, 90)}: ${r.status} ${text.slice(0, 120).replace(/\n/g, ' ')}`);
    if (r.status === 401 || r.status === 403) throw new Error(`${base}: not logged in (${jar})`);
    return { status: r.status, text };
  };
}

// what a trace's driver gets: rec, the engine's calls; hus, ~hus's HTTP;
// note(name, ms), the next event of that name
function context(rec, events) {
  return {
    rec,
    call: (op, ...args) => rec.call(op, ...args),
    hus: ship(HUS, opt.hus),
    grubbery: ship('http://localhost:8134/grubbery/api', opt.hus),
    sleep,
    // the next event named, or null after ms
    note: (name, ms) => new Promise(resolve => {
      const t = setTimeout(() => resolve(null), ms);
      events.push({ name, resolve: v => { clearTimeout(t); resolve(v); } });
    }),
    // poll f every 2 s until it gives something, for up to ms
    async until(f, ms = 60_000) {
      for (const end = Date.now() + ms; Date.now() < end; await sleep(2000)) {
        const v = await f();
        if (v) return v;
      }
      return null;
    },
    open: async (who, more = {}) => rec.call('open', { keyfile: who && await fakeKeyfile(wasm, who), fresh: true, ...more.load }, { fake: true, ...more.opts }),
  };
}

const settle = p => p.then(v => ({ ok: v }), e => ({ err: e.message }));
const PAGE = `${DESK_BASE}/page`;

const TRACES = {
  1: { name: 'cold-comet', desc: 'a cold comet reads a lattice page, the home page, a 15 KB page and a missing page',
    async run(c) {
      const { our } = await c.open(null);
      log(`  mined ${our}`);
      for (const url of ['urb://~hus/fleet-check', 'urb://~hus', 'urb://~hus/big', 'urb://~hus/no-such-page']) {
        log(`  ${url}: ${JSON.stringify(await settle(c.call('read', url).then(r => `${r.key} ${r.how} ${r.mark} ${r.body.length}`)))}`);
      }
      await c.call('close');
    } },
  2: { name: 'moon-runs', desc: "a moon's first run (fresh), a second from its saved state, and a third from empty state, refused",
    async run(c, rec) {
      await c.open(MOON(2));
      log(`  ${(await c.call('read', 'urb://~hus')).key}`);
      await c.call('close');
      await c.open(MOON(2), { load: { rows: rec.rows(), fresh: false } });
      log(`  ${(await c.call('read', 'urb://~hus/fleet-check')).key}`);
      await c.call('close');
      //  a moon that ran, off the fake network, from nothing: refused
      log(`  ${JSON.stringify(await settle(c.open(MOON(2), { load: { fresh: false }, opts: { fake: false } })))}`);
    } },
  3: { name: 'private-reads', desc: 'private reads: a know entry and a tree file, each granted and not',
    async run(c) {
      await c.open(MOON(3));
      for (const url of ['urb://~hus/k/fleet/granted', 'urb://~hus/k/fleet/withheld',
                         `urb://~hus/t${PAGE}/fleet-check/code`, `urb://~hus/t${PAGE}/big/code`]) {
        log(`  ${url}: ${JSON.stringify(await settle(c.call('read', url).then(r => `${r.key} ${r.body.slice(0, 40)}`)))}`);
      }
      await c.call('close');
    } },
  4: { name: 'furum', desc: 'furum on ~rup: a list, a thread, a post, a comment, an upvote and a refusal note',
    async run(c) {
      const { our } = await c.open(MOON(4));
      await c.call('furum', '~rup', 'onNote');
      const before = await c.call('furum', '~rup', 'posts', 'm1c');
      log(`  ${before.length} posts`);
      log(`  thread 0: ${(await c.call('furum', '~rup', 'comments', 'm1c', 0)).length} comments, ${JSON.stringify(await c.call('furum', '~rup', 'votes', 'm1c', 0))}`);
      log(`  post: ${JSON.stringify(await c.call('furum', '~rup', 'newPost', 'm1c', { title: 'A trace-4 post', body: 'from the recorder' }))}`);
      const mine = await c.until(async () => (await c.call('furum', '~rup', 'posts', 'm1c')).find(p => p.author === our));
      log(`  on the board as #${mine?.id}`);
      //  a comment within furum's 10 s cooldown: refused, with a note
      const note = c.note('furum.note', 60_000);
      log(`  comment at once: ${JSON.stringify(await c.call('furum', '~rup', 'newComment', 'm1c', 0, 'too soon'))}`);
      log(`  note: ${JSON.stringify(await note)}`);
      await sleep(12_000);
      log(`  comment: ${JSON.stringify(await c.call('furum', '~rup', 'newComment', 'm1c', 0, 'a trace-4 comment'))}`);
      log(`  landed: ${Boolean(await c.until(async () => (await c.call('furum', '~rup', 'comments', 'm1c', 0)).some(x => x.author === our)))}`);
      log(`  upvote: ${JSON.stringify(await c.call('furum', '~rup', 'upvote', 'm1c', 0))}`);
      log(`  landed: ${Boolean(await c.until(async () => (await c.call('furum', '~rup', 'votes', 'm1c', 0)).up.includes(our)))}`);
      await c.call('close');
    } },
  5: { name: 'publishing', desc: "publishing: lattice on ~hus peeks the index and revs, keens, keeps; an edit, a removal, a republish, and its drop",
    async run(c) {
      const { our } = await c.open(MOON(5));
      //  ~hus learns where we are from our first message: a moon's
      //  sponsor on the fake network is a planet nobody runs
      await c.call('read', 'urb://~hus');
      await c.call('pub', {});
      await c.call('pub.put', 'm1c-page', '# Trace 5\n\nPublished by the recorder.', 'md');
      const url = encodeURIComponent(`urb://${our}/m1c-page`);
      await c.hus('GET', `/fetch?url=${url}`);
      const kept = c.note('pub.save', 60_000);
      await c.hus('POST', `/sub?url=${url}`);
      log(`  keepers: ${JSON.stringify((await kept)?.value?.keepers)}`);
      await sleep(3000);
      await c.call('pub.put', 'm1c-page', '# Trace 5\n\nEdited.', 'md');
      await sleep(5000);
      await c.hus('GET', `/fetch?url=${url}`);
      await c.call('pub.remove', 'm1c-page');
      await sleep(5000);
      await c.hus('GET', `/fetch?url=${url}`);
      await c.call('pub.put', 'm1c-page', '# Trace 5\n\nPublished again.', 'md');
      await sleep(5000);
      await c.hus('GET', `/fetch?url=${url}`);
      await c.hus('POST', `/unsub?url=${url}`);
      //  /unsub culls the grub that kept it; grubbery drops the keep when
      //  the next wave finds nobody watching (+take-wave's self-heal)
      await sleep(3000);
      const dropped = c.note('pub.save', 60_000);
      await c.call('pub.put', 'm1c-page', '# Trace 5\n\nAfter the drop.', 'md');
      log(`  keepers after the edit: ${JSON.stringify((await dropped)?.value?.keepers)}`);
      const gone = c.note('pub.save', 60_000);
      log(`  keepers after the drop: ${JSON.stringify((await gone)?.value?.keepers)}`);
      await c.call('close');
    } },
  6: { name: 'timeouts', desc: 'timeouts: a keen that misses and is yawned, a peek nobody answers, a poke with no %pack',
    async run(c) {
      await c.hus('POST', '/page-save?name=m1c-stale&type=gmi', '# Listed under an id nothing answers');
      await c.hus('POST', '/page-share?name=m1c-stale&mode=urbit');
      //  the revision list names an id the page was never grown at, as a
      //  list written ahead of its binding would: a keen there misses
      const revs = JSON.parse((await c.grubbery('GET', `/file${DESK_BASE}/pub/revs`)).text);
      revs['/m1c-stale'].id = '0v1';
      await c.grubbery('POST', `/over${DESK_BASE}/pub/revs?blot=/json`, JSON.stringify(revs));
      await c.open(MOON(6));
      log(`  ${JSON.stringify(await settle(c.call('read', 'urb://~hus/m1c-stale').then(r => `${r.how} ${r.body}`)))}`);
      //  ~fes runs nothing: no peek or poke there is answered
      log(`  ${JSON.stringify(await settle(c.call('read', 'urb://~fes/page')))}`);
      log(`  ${JSON.stringify(await c.call('poke', '~fes', `${DESK_BASE}`, 'forum.sig', ['/lattice', 'forum-action'], ['upvote', 'x', ['post', 1]]))}`);
      await c.call('close');
    } },
  7: { name: 'want-leg', desc: 'a page over 8 KB from real grubbery, through the %want leg',
    async run(c) {
      await c.open(MOON(7));
      const body = await c.call('peek', '~hus', `${DESK_BASE}/pub/vault/big`, 'gmi', 'cord');
      log(`  the vault file by peek: ${body?.length} characters`);
      log(`  by read: ${(await c.call('read', 'urb://~hus/big')).how}`);
      await c.call('close');
    } },
  8: { name: 'lattice-forum', desc: 'a lattice forum over urb://: a board, a thread, a post that lands, and one refused with its note',
    async run(c) {
      const { our } = await c.open(MOON(8));
      await c.call('forum', '~hus', 'onNote');
      const b = await c.call('forum', '~hus', 'board', 'm1btest');
      log(`  ${b.info.title}: ${b.posts.length} posts`);
      log(`  thread ${b.posts.at(-1).id}: ${(await c.call('forum', '~hus', 'thread', 'm1btest', b.posts.at(-1).id)).comments.length} comments`);
      log(`  post: ${JSON.stringify(await c.call('forum', '~hus', 'newPost', 'm1btest', { title: 'A trace-8 post', body: 'from the recorder' }))}`);
      const note = c.note('forum.note', 60_000);
      log(`  at once, another: ${JSON.stringify(await c.call('forum', '~hus', 'newPost', 'm1btest', { title: 'Too soon' }))}`);
      log(`  note: ${JSON.stringify(await note)}`);
      const mine = await c.until(async () => (await c.call('forum', '~hus', 'board', 'm1btest')).posts.find(p => p.author === our));
      log(`  landed as #${mine?.id}`);
      await c.call('close');
    } },
  9: { name: 'marks-and-salt', desc: 'md, html and text pages from lattice 81, and a page regrown under a rotated salt',
    async run(c) {
      await c.open(MOON(9));
      for (const m of ['md', 'html', 'text', 'gmi', 'css']) {
        const r = await c.call('read', `urb://~hus/marks/${m}`);
        log(`  marks/${m}: %${r.mark} by ${r.how}`);
      }
      //  a new salt, and every page regrown under it: new ids
      const salt = BigInt('0x' + createHash('sha256').update(String(Date.now())).digest('hex').slice(0, 30)).toString();
      await c.grubbery('POST', `/over${DESK_BASE}/salt?blot=/ud`, salt);
      await c.hus('POST', '/pub-regrow');
      log(`  within the cache: ${(await c.call('read', 'urb://~hus/marks/md')).how}`);
      await sleep(61_000);
      log(`  past it: ${(await c.call('read', 'urb://~hus/marks/md')).how}`);
      await c.call('close');
    } },
};

const which = positionals.length ? positionals.map(Number) : Object.keys(TRACES).map(Number);
const wasmSha = createHash('sha256').update(wasm).digest('hex');
mkdirSync(opt.out, { recursive: true });
let failed = 0;
for (const n of which) {
  const t = TRACES[n];
  if (!t) throw new Error(`no trace ${n}`);
  log(`trace ${n}: ${t.desc}`);
  const net = await udp(), events = [];
  const rec = recorder({ wasm, transport: net, onOut: o => {
    if (!o.event) return;
    const i = events.findIndex(e => e.name === o.event);
    if (i >= 0) events.splice(i, 1)[0].resolve(o);
  } });
  try {
    await t.run(context(rec, events), rec);
  } catch (e) {
    log(`  FAILED: ${e.stack}`);
    failed++;
    if (rec.urb) await rec.call('close').catch(() => {});
  }
  net.end();
  const trace = rec.trace({
    trace: n, name: t.name, desc: t.desc, recorded: new Date().toISOString(),
    fleet: 'furum set, snapshot m1c-traces: ~rup furum 7, ~hus lattice 81, grubbery eed73a6',
    wasm: wasmSha,
  });
  const file = `${opt.out}/${n}-${t.name}.jsonl`;
  writeFileSync(file, toLines(trace));
  log(`  ${trace.steps.length} steps -> ${file}`);
}
process.exit(failed ? 1 : 0);
