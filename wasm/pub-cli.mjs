// Publishes pages from Node for lattice readers on other ships (pub.mjs on
// mesa.mjs), and serves them until Ctrl-C:
//
//   node pub-cli.mjs [node.mjs's options] [--put NAME=FILE]... [--remove NAME]...
//
// A page's format, the mark it is published under, comes from FILE's
// extension, in any case: .md, .html, .txt, .js, .css, else gemtext.
//
// A reader opens urb://~our-ship/NAME. The pages, and the keeps of readers
// following them, are kept in pages.json in the identity's state folder,
// so a later run serves them again. On a fake network, for a fleet set:
//   node pub-cli.mjs --fake ~bidlys-donref-dozzod-dozrup --put notes=notes.gmi
//
// It logs what other ships ask for: each file looked up, each keep, each
// scry.

import { readFileSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { DESK_BASE } from './mesa.mjs';
import { connect, log, options } from './node.mjs';
import { markOf, publish } from './pub.mjs';

const { values: opt } = parseArgs({
  options: { ...options, put: { type: 'string', multiple: true, default: [] }, remove: { type: 'string', multiple: true, default: [] } },
});

// a run that stops before its last acks leave stalls the ships it spoke
// to, so it closes the engine on every way out
let urb = null;
try {
  urb = await connect(opt);
  //  say what other ships ask for: each file looked up (a peek, a keep, or
  //  news to keepers), each keep made or dropped, each scry
  const { serve } = urb.grub, scry = urb.scry.serve;
  urb.grub.serve = (f, saved, onKeepers) => serve(
    (folder, name) => {
      const got = f(folder, name);
      log(`file ${folder.replace(DESK_BASE, '<base>')}/${name}: ${got ? `rev ${got.rev}` : 'not ours'}`);
      return got;
    },
    saved,
    ks => {
      log(`keeps: ${ks.map(k => `${k.ship} ${k.folder.replace(DESK_BASE, '<base>')}/${k.name}`).join(', ') || 'none'}`);
      onKeepers(ks);
    },
  );
  urb.scry.serve = f => scry(p => {
    const got = f(p);
    log(`scry ${p}: ${got ? 'answered' : 'nothing'}`);
    return got;
  });
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
    const at = p.indexOf('='), file = p.slice(at + 1);
    const rec = at > 0 && pub.put(p.slice(0, at), readFileSync(file, 'utf8'), markOf(file));
    if (!rec) throw new Error(`--put ${p}: NAME=FILE, NAME as lattice names pages (notes/intro)`);
  }
  for (const name of opt.remove) if (!pub.remove(name)) log(`--remove ${name}: no such page`);
  for (const [key, p] of Object.entries(pub.pages())) {
    if (p.body !== null) log(`urb://${urb.our}${key}  %${p.mark ?? 'gmi'} rev ${p.rev}, ${p.body.length} characters`);
  }
} catch (e) {
  log(e.message || e);
  await urb?.close();
  process.exit(1);
}
log('serving; Ctrl-C stops');
process.on('SIGINT', () => urb.close().then(() => process.exit(0)));
