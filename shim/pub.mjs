// pub.mjs on the Rust core (docs/rust-core.md, M6): lattice's publishing
// layout, served by the core in mesa.wasm (core/urb/src/publish.rs), with
// wasm/pub.mjs's interface.
//
//   const pub = publish(urb, { pages, keepers, save })
//   pub.put('notes/intro', '# Hello', 'md')   // its record, or null for a bad name
//   pub.remove('notes/intro')                 // true if it was published
//   pub.pages()                               // {'/<name>': {body, rev, id, at, hash, mark}}
//
// pages and keepers are meta.pub as it was saved, and save({pages,
// keepers}) hears each change, to keep. Every call answers at once, as
// before.

import { CORE } from './mesa.mjs';
import { call } from './render.js';

// '/<name>' for a page's name ('notes/intro', '/notes/intro/'), or null.
// lattice reserves one-letter first segments for urb:// mounts.
export const pageKey = name => call('render_page_key', [String(name)]);

export function publish(urb, { pages = {}, keepers = [], save = () => {} } = {}) {
  const c = urb[CORE];
  c.on['pub.save'] = save;
  c.callSync('pub', { pages, keepers });
  return {
    pages: () => c.callSync('pub.pages'),
    put: (name, body, mark = 'gmi') => c.callSync('pub.put', name, body, mark),
    remove: name => c.callSync('pub.remove', name),
  };
}
