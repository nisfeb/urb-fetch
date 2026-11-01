// After `fleet reset furum <snapshot>`, ~hus's serf runs flat out for
// about 37 s, from about 64 s to 101 s after it starts, whether or not
// anyone talks to it (measured for gate 2, 2026-11-01: no traffic at
// all, and the same burst). It happens once per boot of the snapshot. A
// read that starts then gets no answer within its two 8 s base probes
// ("~hus answered no lattice peek"), for the Node and the Rust engine
// alike. That was M7's "flake after some 28 new moons": the 28th moon
// of cli-diff's read-only phase is about a minute after the reset. With
// 4 s between moons it came at the 10th instead, and after 150 s of
// idling it never came.
//
// settle() waits for it to pass: until each serf is at least MIN_AGE
// old and has used under a fifth of a core over 3 s. It is synchronous,
// as the scripts' reset() is.

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const HZ = 100;         // USER_HZ, the unit of /proc/<pid>/stat's times
const MIN_AGE = 110;    // s: past the burst's end, with room
const LIMIT = 300_000;  // ms: give up, loudly
const nap = ms => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

function serf(ship) {
  const r = spawnSync('pgrep', ['-f', `work --snap-dir ${process.env.HOME}/fleet/piers/${ship} `], { encoding: 'utf8' });
  return Number(r.stdout.split('\n')[0]) || null;
}
// [age in s, cpu in ticks]
function stat(pid) {
  const f = readFileSync(`/proc/${pid}/stat`, 'utf8');
  const s = f.slice(f.lastIndexOf(')') + 2).split(' ');   // from field 3
  const up = Number(readFileSync('/proc/uptime', 'utf8').split(' ')[0]);
  return [up - Number(s[19]) / HZ, Number(s[11]) + Number(s[12])];
}

export function settle(ships = ['hus', 'rup'], say = console.log) {
  const t = Date.now();
  for (const ship of ships) {
    for (;;) {
      if (Date.now() - t > LIMIT) throw new Error(`settle: ~${ship} is still busy ${LIMIT / 1000} s after the reset`);
      const pid = serf(ship);
      if (!pid) throw new Error(`settle: no serf for ~${ship}`);
      const [age, c0] = stat(pid);
      if (age < MIN_AGE) { nap(Math.min(5000, (MIN_AGE - age) * 1000 + 50)); continue; }
      nap(3000);
      const [, c1] = stat(pid);
      if (c1 - c0 < 0.2 * 3 * HZ) break;
    }
  }
  say(`settle: waited ${((Date.now() - t) / 1000).toFixed(0)} s for ~${ships.join(' and ~')} to finish booting`);
}
