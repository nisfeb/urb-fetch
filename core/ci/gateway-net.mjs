#!/usr/bin/env node
// The net cases of gateway-diff.mjs (docs/rust-core.md, M8's fix
// round), which need no ship: run inside a network namespace with only
// loopback (gateway-diff runs it under `unshare -rn`). The Go gateway
// on 127.0.0.2 and the Rust one on 127.0.0.3, each as the fake ~zod
// with nothing to answer it, get:
//
//   - requests written raw that their HTTP servers refuse or take, each
//     on a connection of its own: compared whole when refused, and with
//     the framing headers dropped (Date, Content-Length, Connection,
//     Transfer-Encoding) when answered;
//   - slow clients: a connection that sends nothing, one kept alive
//     after its answer, a head or a body sent a byte at a time, a body
//     announced and never sent. What each side said must be the same,
//     and each must close the connection within half a second of the
//     other (Go's 10 s timeouts);
//   - a flood: 300 idle and 30 trickling connections at once, during
//     which a request must still answer within a second, and all of
//     them must be closed by 12 s;
//   - two startup failures that need the namespace: the kept UDP port
//     in use, and a comet mined with no network (the transport's own
//     words after `Get "<url>": ` are masked: Go's resolver and ureq's
//     say it differently).
//
//   node gateway-net.mjs GO RUST OUTDIR
//
// It prints [name, same, go, rust] for each case as JSON.

import { spawn, spawnSync } from 'node:child_process';
import { createSocket } from 'node:dgram';
import { mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import { connect } from 'node:net';
import { join } from 'node:path';

const [GO, RUST, out] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const AT = { go: '127.0.0.2', rust: '127.0.0.3' };
const PORT = 8475;
const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = (...a) => console.error(...a);

if (spawnSync('ip', ['link', 'set', 'lo', 'up']).status !== 0) throw new Error('ip link set lo up failed');

async function start(side) {
  const p = spawn(side === 'go' ? GO : RUST, ['-fake', '~zod', '-state', join(out, `state-${side}`), '-listen', `${AT[side]}:${PORT}`]);
  let text = '';
  p.stdout.on('data', d => { text += d; });
  p.stderr.on('data', d => { text += d; });
  const t = Date.now();
  while (!text.includes('serving http://') && p.exitCode === null && Date.now() - t < 30_000) await sleep(50);
  if (!text.includes('serving http://')) throw new Error(`${side} did not start: ${text}`);
  return p;
}

// a connection: send bytes at once, then slow one byte every trickle ms
// (an EOF after them if end), and resolve with what came back and when
// it closed, in seconds from the connect
function talk(ip, bytes, { slow = Buffer.alloc(0), trickle = 0, end = false, limit = 30_000 } = {}) {
  return new Promise(resolve => {
    const chunks = [];
    let t0 = 0, timer = null, done = false;
    const s = connect({ host: ip, port: PORT }, async () => {
      t0 = performance.now();
      if (bytes.length) s.write(bytes);
      for (const b of slow) {
        await sleep(trickle);
        if (done) return;
        s.write(Buffer.from([b]));
      }
      if (end) s.end();
    });
    timer = setTimeout(() => s.destroy(), limit);
    s.on('data', d => chunks.push(d));
    const fin = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve({ got: Buffer.concat(chunks).toString('latin1'), t: (performance.now() - t0) / 1000 });
    };
    s.on('close', fin);
    s.on('end', () => s.destroy());
    s.on('error', () => {});
  });
}

const FRAMING = /^(date|content-length|transfer-encoding|connection):.*\r\n/gim;
const H = `Host: localhost:${PORT}\r\n`;
const C = 'Connection: close\r\n';
// [name, request, whether Go refuses it before its handler (compared
// whole)]
const RAW = [
  ['raw-no-host', `GET / HTTP/1.1\r\n${C}\r\n`, true],
  ['raw-http10-home', `GET / HTTP/1.0\r\n${H}\r\n`],
  ['raw-bad-host', `GET / HTTP/1.1\r\nHost: zod.urb.localhost:${PORT}"\r\n${C}\r\n`, true],
  ['raw-two-hosts', `GET / HTTP/1.1\r\n${H}${H}${C}\r\n`, true],
  ['raw-http2', `GET / HTTP/2.0\r\n${H}\r\n`, true],
  ['raw-bad-line', 'GARBAGE\r\n\r\n', true],
  ['raw-cl-plus', `GET / HTTP/1.1\r\n${H}Content-Length: +0\r\n${C}\r\n`, true],
  ['raw-cl-empty', `GET / HTTP/1.1\r\n${H}Content-Length: \r\n${C}\r\n`, true],
  ['raw-cl-two', `GET / HTTP/1.1\r\n${H}Content-Length: 0\r\nContent-Length: 5\r\n${C}\r\nhello`, true],
  ['raw-cl-two-same', `GET / HTTP/1.1\r\n${H}Content-Length: 0\r\nContent-Length: 0\r\n${C}\r\n`],
  ['raw-te-gzip', `GET / HTTP/1.1\r\n${H}Transfer-Encoding: gzip\r\n${C}\r\n`, true],
  ['raw-te-two', `GET / HTTP/1.1\r\n${H}Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n${C}\r\n0\r\n\r\n`, true],
  ['raw-te-http10', `GET / HTTP/1.0\r\n${H}Transfer-Encoding: gzip\r\n\r\n`],
  ['raw-te-chunked', `GET / HTTP/1.1\r\n${H}Transfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\nGET / HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-trailer-bad', `GET / HTTP/1.1\r\n${H}Transfer-Encoding: chunked\r\nTrailer: content-length\r\n${C}\r\n0\r\n\r\n`, true],
  ['raw-http1-10', `GET / HTTP/1.10\r\n${H}${C}\r\n`, true],
  ['raw-http0-9', `GET / HTTP/0.9\r\n${H}\r\n`, true],
  ['raw-http1-5', `GET / HTTP/1.5\r\n${H}${C}\r\n`],
  ['raw-options-star', `OPTIONS * HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-options-path', `OPTIONS / HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-get-star', `GET * HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-connect', `CONNECT zod.urb.localhost:${PORT} HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-expect-other', `GET / HTTP/1.1\r\n${H}Expect: tea\r\n\r\n`],
  ['raw-expect-continue', `GET / HTTP/1.1\r\n${H}Expect: 100-continue\r\nContent-Length: 3\r\n${C}\r\nabc`],
  ['raw-obs-fold', `GET / HTTP/1.1\r\n${H}X-A: a\r\n b\r\n${C}\r\n`],
  ['raw-space-in-name', `GET / HTTP/1.1\r\n${H}X A: b\r\n${C}\r\n`, true],
  ['raw-ctl-in-value', `GET / HTTP/1.1\r\n${H}X-A: a\x01b\r\n${C}\r\n`, true],
  ['raw-leading-space', `GET / HTTP/1.1\r\n X: y\r\n${H}${C}\r\n`, true],
  ['raw-no-colon', `GET / HTTP/1.1\r\n${H}Nocolon\r\n${C}\r\n`, true],
  ['raw-many-headers', `GET / HTTP/1.1\r\n${H}${Array.from({ length: 600 }, (_, i) => `X-${i}: 1\r\n`).join('')}${C}\r\n`, true],
  ['raw-lower-host', `GET / HTTP/1.1\r\nhost: localhost:${PORT}\r\n${C}\r\n`],
  ['raw-lf-only', `GET / HTTP/1.1\n${H.replace('\r\n', '\n')}Connection: close\n\n`],
  ['raw-keepalive-two', `GET / HTTP/1.1\r\n${H}\r\nGET /go?url=urb://~zod/x HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-post-then-get', `POST / HTTP/1.1\r\n${H}Content-Length: 2\r\n\r\nab\r\nGET / HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-body-keepalive', `GET / HTTP/1.1\r\n${H}Content-Length: 3\r\n\r\nabcGET / HTTP/1.1\r\n${H}${C}\r\n`],
  ['raw-half-request', `GET / HTTP/1.1\r\nHost: localhost:${PORT}`, true, true],
];
const GET = `GET / HTTP/1.1\r\n${H}`;
// [name, sent at once, then sent a byte every 150 ms]
const SLOW = [
  ['slow-idle', '', ''],
  ['slow-kept-alive', `${GET}\r\n`, ''],
  ['slow-trickled-head', '', `${GET}X-Pad: ${'a'.repeat(200)}\r\n\r\n`],
  ['slow-unsent-body', `${GET}Content-Length: 10\r\n\r\n`, ''],
  ['slow-unsent-body-close', `${GET}Content-Length: 10\r\n${C}\r\n`, ''],
  ['slow-trickled-body', `${GET}Content-Length: 100\r\n\r\n`, 'x'.repeat(100)],
];

const results = [];

// startup: exit code and stderr, the log's time and the state folder
// masked
const stamp = t => t.replace(/^\d{4}\/\d\d\/\d\d \d\d:\d\d:\d\d /gm, '<time> ');
function startup(name, args, prep = () => {}, mask = t => t) {
  const one = side => {
    const st = join(out, `${name}-${side}`);
    mkdirSync(st, { recursive: true });
    prep(st);
    const p = spawnSync(side === 'go' ? GO : RUST, ['-state', st, '-listen', `${AT[side]}:${PORT}`, ...args], { encoding: 'utf8', timeout: 60_000 });
    return `exit ${p.status ?? p.signal}\n${mask(stamp(p.stderr.split(st).join('<state>')))}`;
  };
  const [a, b] = [one('go'), one('rust')];
  results.push([name, a === b, a, b]);
}
{
  // the port fake-zod.db.port keeps, held by another socket
  const held = createSocket('udp4');
  await new Promise(ok => held.bind(40_001, '127.0.0.1', ok));
  startup('start-udp-in-use', ['-fake', '~zod'], st => writeFileSync(join(st, 'fake-zod.db.port'), '40001\n'));
  held.close();
  startup('start-comet-offline', [], undefined, t => t.replace(/(comet-stars\.jam": ).*/, '$1<the transport\'s words>'));
}

const g = await start('go'), r = await start('rust');
try {
  for (const [name, req, refused, end] of RAW) {
    const [a, b] = await Promise.all(['go', 'rust'].map(s => talk(AT[s], Buffer.from(req, 'latin1'), { end: end ?? true })));
    const shown = x => (refused ? x.got : x.got.replace(FRAMING, ''));
    results.push([name, shown(a) === shown(b), shown(a), shown(b)]);
  }
  const slow = await Promise.all(SLOW.flatMap(([name, now, later]) => ['go', 'rust'].map(s =>
    talk(AT[s], Buffer.from(now, 'latin1'), { slow: Buffer.from(later, 'latin1'), trickle: 150 }).then(x => [name, s, x]))));
  for (const [name] of SLOW) {
    const [a, b] = ['go', 'rust'].map(s => slow.find(x => x[0] === name && x[1] === s)[2]);
    const shown = x => `${x.got.replace(FRAMING, '')}\n(closed at ${x.t.toFixed(2)} s)\n`;
    results.push([name, a.got.replace(FRAMING, '') === b.got.replace(FRAMING, '') && Math.abs(a.t - b.t) < 0.5, shown(a), shown(b)]);
  }
  const flood = async side => {
    const pid = side === 'go' ? g.pid : r.pid;
    const held = [
      ...Array.from({ length: 300 }, () => talk(AT[side], Buffer.alloc(0))),
      ...Array.from({ length: 30 }, () => talk(AT[side], Buffer.alloc(0), { slow: Buffer.from(`${GET}X-Pad: ${'a'.repeat(400)}\r\n\r\n`), trickle: 100 })),
    ];
    await sleep(500);
    const threads = readdirSync(`/proc/${pid}/task`).length;
    const home = await talk(AT[side], Buffer.from(`${GET}${C}\r\n`));
    const closed = await Promise.all(held);
    const late = closed.filter(x => x.t > 12).length;
    if (late) log(`  flood, ${side}: closed late`, JSON.stringify(closed.map((x, i) => [i, +x.t.toFixed(2)]).filter(x => x[1] > 12)));
    log(`  flood, ${side}: ${threads} threads with 330 connections held`);
    return { ok: home.got.startsWith('HTTP/1.1 200 OK') && home.t < 1 && late === 0,
      text: `home: ${home.got.slice(0, 15)} in ${home.t < 1 ? 'under' : 'over'} 1 s\nclosed after 12 s: ${late} of ${closed.length}\n` };
  };
  const [fa, fb] = await Promise.all([flood('go'), flood('rust')]);
  results.push(['flood', fa.ok && fb.ok && fa.text === fb.text, fa.text, fb.text]);
} finally {
  g.kill('SIGINT');
  r.kill('SIGINT');
}
console.log(JSON.stringify(results));
