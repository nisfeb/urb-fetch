// Publishes pages from Node for lattice readers on other ships (pub.mjs on
// mesa.mjs), and serves them until Ctrl-C:
//
//   node pub-cli.mjs [node.mjs's options] [--put NAME=FILE]... [--remove NAME]...
//
// A reader opens urb://~our-ship/NAME. The pages, and the keeps of readers
// following them, are kept in pages.json in the identity's state folder,
// so a later run serves them again. On a fake network, for a fleet set:
//   node pub-cli.mjs --fake ~doznec-dozzod-dozhus --put notes=notes.gmi

import { readFileSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { connect, log, options } from './node.mjs';
import { publish } from './pub.mjs';

const { values: opt } = parseArgs({
  options: { ...options, put: { type: 'string', multiple: true, default: [] }, remove: { type: 'string', multiple: true, default: [] } },
});

// a run that stops before its last acks leave stalls the ships it spoke
// to, so it closes the engine on every way out
let urb = null;
try {
  urb = await connect(opt);
  const file = join(urb.dir, 'pages.json');
  let saved = {};
  try { saved = JSON.parse(readFileSync(file, 'utf8')); } catch {}
  const pub = publish(urb, {
    ...saved,
    save(all) {
      writeFileSync(file + '.new', JSON.stringify(all), { mode: 0o600 });
      renameSync(file + '.new', file);
    },
  });
  for (const p of opt.put) {
    const at = p.indexOf('=');
    const rec = at > 0 && pub.put(p.slice(0, at), readFileSync(p.slice(at + 1), 'utf8'));
    if (!rec) throw new Error(`--put ${p}: NAME=FILE, NAME as lattice names pages (notes/intro)`);
  }
  for (const name of opt.remove) if (!pub.remove(name)) log(`--remove ${name}: no such page`);
  for (const [key, p] of Object.entries(pub.pages())) {
    if (p.body !== null) log(`urb://${urb.our}${key}  rev ${p.rev}, ${p.body.length} characters`);
  }
} catch (e) {
  log(e.message || e);
  await urb?.close();
  process.exit(1);
}
log('serving; Ctrl-C stops');
process.on('SIGINT', () => urb.close().then(() => process.exit(0)));
