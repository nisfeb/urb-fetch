// relayTransport against relays that don't work, and closed while down
// or connecting (relay-cases.mjs, which the shim runs too).

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { relayTransport } from '../mesa.mjs';
import { relayCases } from './relay-cases.mjs';

for (const [what, run] of relayCases) {
  test(what, async () => {
    const failed = await run(relayTransport);
    assert.equal(failed, undefined, failed);
  });
}
