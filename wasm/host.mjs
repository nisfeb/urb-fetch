// Reads one page published with lattice from Node, as a comet, a moon or a
// fake ship, run by libmesa compiled to wasm (mesa.mjs, build/mesa.wasm):
//
//   node host.mjs [--tries N] [node.mjs's options] urb://~ship/page
//
// The body goes to stdout, as published; stderr says its mark and how it
// renders (mesa.mjs's renderAs). The comet is mined on the first run and kept in
// the state folder with the engine's state, so later runs are the same
// comet. node.mjs says how to run as a moon or on a fake network.

import { parseArgs } from 'node:util';
import { UrbError, renderAs } from './mesa.mjs';
import { connect, log, options } from './node.mjs';

const { values: opt, positionals: [url] } = parseArgs({
  allowPositionals: true,
  options: { ...options, tries: { type: 'string', default: '1' } },
});
if (!url) {
  log('usage: node host.mjs [--state DIR] [--relay URL | --fake ~SHIP] [--moon KEYFILE [--new-moon]] [--tries N] [--trace] urb://~ship/page');
  process.exit(2);
}

let code = 0, urb = null;
try {
  urb = await connect(opt);
  const t = Date.now();
  //  a ship can take a minute or two to answer a comet that moved or
  //  restarted: it is still resending answers meant for earlier runs, in
  //  order, and the engine keeps running between tries
  let got;
  for (let i = 1; !got; i++) {
    try {
      got = await urb.read(url);
    } catch (e) {
      if (!(e instanceof UrbError) || i >= Number(opt.tries)) throw e;
      log(`try ${i}: ${e.message}`);
    }
  }
  const { key, body, how, mark } = got;
  process.stdout.write(body);
  log(`ok ${url} -> ${key}${mark ? ` %${mark}, renders as ${renderAs(mark, body)}` : ''}, ${new TextEncoder().encode(body).length} bytes, by ${how}, ${((Date.now() - t) / 1000).toFixed(1)}s`);
} catch (e) {
  log(e instanceof UrbError ? `FAIL ${e.message}` : e);
  code = 1;
} finally {
  await urb?.close();
}
process.exit(code);
