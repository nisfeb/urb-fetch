// pub.mjs: pages published from a libmesa ship (mesa.mjs), for lattice's
// readers on other ships to read at urb://~our-ship/<name>. It lays them
// out as lattice publishes its own (nisfeb/lattice, nex/lattice/app.hoon):
//
//   <base>/pub index             grubbery peek: where we run lattice
//                                (+peer-base), and the page list
//   <base>/pub revs              grubbery peek: each page's rev and id
//   <base>/pub/vault/<name> gmi  grubbery peek, the slow path; a keep here
//                                follows the page (/sub/pages)
//   /g/x/1/grubbery//1/pub/page/<name>/<id>   remote scry: the body
//
//   const pub = publish(urb, { pages, keepers, save })
//   pub.put('notes/intro', '# Hello')
//
// pages is {'/<name>': {body, rev, id, at, hash}}: body null once removed,
// where the rev stays, so a page published again counts on from it. Each
// put and remove hands {pages, keepers} to save, plain JSON to keep for
// the next run.

import { DESK_BASE as BASE, da, uv } from './mesa.mjs';

const enc = new TextEncoder();
const PAGE = '/g/x/1/grubbery//1/pub/page';   // +keen-path
const KNOT = /^[a-z0-9~._-]+$/;

// '/<name>' for a page's name ('notes/intro', '/notes/intro/'), or null.
// lattice reserves one-letter first segments for urb:// mounts.
export function pageKey(name) {
  const segs = String(name).trim().replace(/^\/+|\/+$/g, '').split('/');
  if (!segs.every(s => KNOT.test(s)) || segs[0].length < 2) return null;
  return '/' + segs.join('/');
}

export function publish(urb, { pages = {}, keepers = [], save = () => {} } = {}) {
  const pubDir = BASE + '/pub', vault = pubDir + '/vault';
  const live = () => Object.entries(pages).filter(([, p]) => p.body !== null);
  const saveAll = () => save({ pages, keepers });

  // the list and the index change with every page: their version is the
  // count of every page's revisions
  const listed = () => {
    const all = Object.values(pages);
    return { rev: all.reduce((n, p) => n + p.rev, 0), at: Math.max(0, ...all.map(p => p.at)) };
  };

  // +revs-json: {"/<name>": {"rev": n, "id": "0v…"}} as a $json noun
  const revs = () => ['o', new Map(live().map(([k, p]) =>
    [k, ['o', new Map([['rev', ['n', String(p.rev)]], ['id', ['s', p.id]]])]])
  )];

  // +pub-index: {/pub/<name>/gmi: [updated bytes hash]}, hash (sham body)
  const index = () => new Map(live().map(([k, p]) =>
    [['pub', ...k.slice(1).split('/'), 'gmi', null], [da(p.at), enc.encode(p.body).length, BigInt('0x' + p.hash)]]));

  function file(folder, name) {
    if (folder === pubDir && name === 'index') return { mark: ['/lattice', 'pub-index'], value: index(), ...listed() };
    if (folder === pubDir && name === 'revs') return { mark: ['/', 'json'], value: revs(), ...listed() };
    if (name !== 'gmi' || !folder.startsWith(vault + '/')) return null;
    const key = pageKey(folder.slice(vault.length));
    if (!key) return null;
    const p = pages[key];
    if (!p) return { rev: 0, at: 0 };   // not yet: a keep waits for it
    return p.body === null ? { rev: p.rev, at: p.at } : { mark: ['/lattice', 'page'], value: p.body, rev: p.rev, at: p.at };
  }

  // the body at the id the list gives. ponytail: only the newest revision
  // answers, so a reader holding an older id (its list is cached a minute)
  // waits out its keen and reads the vault file instead
  function scry(path) {
    if (!path.startsWith(PAGE + '/')) return null;
    const cut = path.lastIndexOf('/'), p = pages[path.slice(PAGE.length, cut)];
    return p && p.body !== null && p.id === path.slice(cut + 1) ? ['gmi', p.body] : null;
  }

  urb.grub.serve(file, keepers, ks => {
    keepers = ks;
    saveAll();
  });
  urb.scry.serve(scry);

  function tell(key) {
    urb.grub.changed(vault + key, 'gmi');
    urb.grub.changed(pubDir, 'revs');
    urb.grub.changed(pubDir, 'index');
  }

  return {
    pages: () => pages,
    // publish [body] as the page [name], a new revision with a new,
    // unguessable id; the record, or null for a name lattice can't read
    put(name, body) {
      const key = pageKey(name);
      if (!key) return null;
      const [hi, lo] = crypto.getRandomValues(new BigUint64Array(2));
      const id = uv((hi << 64n) | lo);
      const hash = urb.nouns.sham(String(body)).toString(16);
      pages[key] = { body: String(body), rev: (pages[key]?.rev ?? 0) + 1, id, at: Date.now(), hash };
      saveAll();
      tell(key);
      return pages[key];
    },
    // take a page down; its rev goes up, so a reader following it hears
    remove(name) {
      const key = pageKey(name), p = key && pages[key];
      if (!p || p.body === null) return false;
      pages[key] = { body: null, rev: p.rev + 1, id: null, at: Date.now(), hash: null };
      saveAll();
      tell(key);
      return true;
    },
  };
}
