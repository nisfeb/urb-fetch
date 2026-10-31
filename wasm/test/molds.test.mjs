// test/grubbery-molds.hoon is grubbery's own text: each block under a
// "::: lib/<file> <from>-<to>" line is those lines of grubbery's
// desk/lib/<file> at the pinned commits. Needs a grubbery checkout
// (GRUBBERY, default ~/software/personal/grubbery); skips without one,
// or fails with REQUIRE_ALL set, as CI sets it.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { test } from 'node:test';

const repo = process.env.GRUBBERY || `${homedir()}/software/personal/grubbery`;
const PINS = ['b126c50', 'eed73a6'];   // ~ricsul-bilwyt's and the fleet's

const missing = !existsSync(`${repo}/.git`) && `no grubbery checkout at ${repo}`;
test('the grubbery molds are copied verbatim', { skip: !process.env.REQUIRE_ALL && missing }, () => {
  assert.ok(!missing, missing);
  const ours = readFileSync(new URL('grubbery-molds.hoon', import.meta.url), 'utf8').split('\n');
  const blocks = [];
  for (const line of ours) {
    const m = /^::: lib\/(\S+) (\d+)(?:-(\d+))?$/.exec(line);
    if (m) blocks.push({ file: m[1], from: +m[2], to: +(m[3] ?? m[2]), lines: [] });
    else if (/^:::/.test(line)) blocks.push(null);   // our own words, or a core's end
    else if (blocks.at(-1)) blocks.at(-1).lines.push(line);
  }
  const copied = blocks.filter(Boolean);
  assert.ok(copied.length >= 10);
  for (const pin of PINS) {
    for (const b of copied) {
      const src = execFileSync('git', ['-C', repo, 'show', `${pin}:desk/lib/${b.file}`], { encoding: 'utf8' }).split('\n');
      const want = src.slice(b.from - 1, b.to);
      assert.deepEqual(b.lines, want, `${pin} lib/${b.file} ${b.from}-${b.to}`);
    }
  }
});
