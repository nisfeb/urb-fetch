// The lattice forum reader: urb:// forum addresses, a board's grubs read
// as lattice's load:fb reads them, hostile grubs, a directory's %snap off
// the wire, and posting and refusal notes between two engines.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DESK_BASE, UrbError, da, decoders, deUrb } from '../mesa.mjs';
import { FORUM_BASES, actions, forum, idOf } from '../forum.mjs';
import { fleet, late, victim } from './fleet.mjs';

test('deUrb: forum/<board>[/<post>] names a board, as lattice 81', () => {
  assert.deepEqual(deUrb('urb://~zod/forum/test'), ['~zod', ['test'], 'forum']);
  assert.deepEqual(deUrb('urb://~zod/forum/test/'), ['~zod', ['test'], 'forum']);
  assert.deepEqual(deUrb('urb://~zod/forum/test/12'), ['~zod', ['test', 12], 'forum']);
  assert.deepEqual(deUrb('urb://~zod/forum/test/0'), ['~zod', ['test', 0], 'forum']);
  //  an @ud as +slaw reads it: dotted past 999
  assert.deepEqual(deUrb('urb://~zod/forum/test/1.000'), ['~zod', ['test', 1000], 'forum']);
  //  any other shape under forum/ is still a page
  for (const [url, rel] of [
    ['urb://~zod/forum', ['forum']],
    ['urb://~zod/forum/test/1000', ['forum', 'test', '1000']],
    ['urb://~zod/forum/test/012', ['forum', 'test', '012']],
    ['urb://~zod/forum/test/x', ['forum', 'test', 'x']],
    ['urb://~zod/forum/test/1/more', ['forum', 'test', '1', 'more']],
  ]) assert.deepEqual(deUrb(url), ['~zod', rel, 'n'], url);
  //  a post past 2^53 - 1 is exact, as a BigInt
  assert.deepEqual(deUrb('urb://~zod/forum/test/9.007.199.254.740.993'), ['~zod', ['test', 9007199254740993n], 'forum']);
  assert.deepEqual(deUrb('urb://~zod/forum/test/9.007.199.254.740.991'), ['~zod', ['test', 9007199254740991], 'forum']);
  //  and /n/ escapes a page named forum/..., as +en-urb writes it
  assert.deepEqual(deUrb('urb://~zod/n/forum/test'), ['~zod', ['forum', 'test'], 'n']);
});

// ------------------------------------------------------------ fake nouns

class Fake {
  constructor(h, t, v) { Object.assign(this, { h, t, v }); }
  get isCell() { return this.v === undefined; }
  get head() { if (!this.isCell) throw new UrbError('atom'); return this.h; }
  get tail() { if (!this.isCell) throw new UrbError('atom'); return this.t; }
  get size() { if (this.isCell) throw new UrbError('cell'); return this.bytes().length; }
  bytes() {
    if (this.isCell) throw new UrbError('cell');
    if (typeof this.v === 'string') return new TextEncoder().encode(this.v);
    const b = [];
    for (let x = BigInt(this.v); x > 0n; x >>= 8n) b.push(Number(x & 0xffn));
    return Uint8Array.from(b);
  }
  text() { return new TextDecoder().decode(this.bytes()); }
  int(max = 32) {
    if (this.size > max) throw new UrbError(`a number over ${max} bytes`);
    return this.bytes().reduceRight((v, x) => (v << 8n) | BigInt(x), 0n);
  }
}
// a JS value as a fake noun: null ~, an array of two or more cells
const N = x => {
  if (x instanceof Fake) return x;
  if (x === null) return new Fake(undefined, undefined, 0);
  if (Array.isArray(x)) return x.slice(0, -1).reduceRight((t, h) => new Fake(N(h), t), N(x.at(-1)));
  return new Fake(undefined, undefined, x);
};
// a treap of [k v] items (or keys, for a set), in any order
const treap = items => (items.length ? [items[0], treap(items.slice(1)), null] : null);
const kv = o => treap(o);

const NEC = 1n, BUD = 2n, WES = 3n;
const t0 = da(Date.UTC(2026, 9, 31, 12));
const board = (over = {}) => ({
  'pub/card': [1, 'test', 'Test board', 'a board', NEC, t0, 'poster', 0],
  'pub/roles': [1, kv([[WES, 'mod']])],
  'pub/conf': [1, 3, null],
  'content/pins': [1, treap([2])],
  'content/sidebar': [1, 'the rules'],
  'content/posts/b0': [1, kv([
    [1, [1, BUD, 'first', null, [null, 'hello'], t0]],
    [2, [2, WES, 'second', [null, 'https://example.com'], null, t0 + (1n << 64n)]],
  ])],
  'content/threads/b0': [1, kv([[1, [3, kv([
    [2, [2, [null, 1], WES, 'a reply', t0]],
    [1, [1, null, NEC, 'a comment', t0]],
  ])]]])],
  'content/votes/b0': [1, kv([
    [['post', 1], [treap([NEC, WES]), null]],
    [['comment', 1, 2], [null, treap([BUD])]],
  ])],
  'content/unknown/grub/here': 'skipped',
  ...over,
});

// an engine stand-in for forum(): the host's files, by folder, as fake
// nouns, and what is poked
async function fakeUrb(files, { indexAt = DESK_BASE } = {}) {
  const f = fleet(), real = await f.engine('~zod', 0);
  const pokes = [], peeks = [];
  const urb = {
    our: '~zod',
    now: () => 1_761_912_000_000,
    nouns: real.nouns,
    grub: {
      async peek(ship, folder, name, conv, wait) {
        peeks.push([folder, name, wait]);
        if (name === 'index') return folder === `${indexAt}/pub` ? conv(N(0)) : null;
        const sub = folder.split('/').at(-1), here = Object.entries(files).filter(([k]) => k.startsWith(sub + '/'));
        if (!folder.startsWith(`${indexAt}/forum/boards/test/`) || !here.length) return null;
        return conv(here.map(([k, v]) => [k.split('/').slice(1), N(v)]));
      },
      async poke(...args) { pokes.push(args); return { ok: true }; },
      onInbox() {},
    },
  };
  return { urb, pokes, peeks, close: () => f.close() };
}

test('a board and a thread, as load:fb assembles them', async () => {
  const u = await fakeUrb(board());
  const fm = forum(u.urb, '~nec');
  const b = await fm.board('test');
  assert.deepEqual(b.info, { name: 'test', title: 'Test board', description: 'a board', host: '~nec',
    created: new Date(Date.UTC(2026, 9, 31, 12)), defaultRole: 'poster', public: true });
  assert.deepEqual(b.roles, { '~wes': 'mod' });
  assert.equal(b.nextPost, 3);
  assert.equal(b.prune, null);
  assert.deepEqual(b.pinned, [2]);
  assert.equal(b.sidebar, 'the rules');
  assert.deepEqual(b.posts.map(p => [p.id, p.author, p.title, p.url, p.body, p.comments, p.up, p.down]), [
    [2, '~wes', 'second', 'https://example.com', null, 0, [], []],
    [1, '~bud', 'first', null, 'hello', 2, ['~nec', '~wes'], []],
  ]);
  assert.equal(b.posts[0].created.getTime() - b.posts[1].created.getTime(), 1000);
  const t = await fm.thread('test', 1);
  assert.equal(t.post.title, 'first');
  assert.deepEqual(t.comments.map(c => [c.id, c.parent, c.author, c.body, c.up, c.down]), [
    [1, null, '~nec', 'a comment', [], []],
    [2, 1, '~wes', 'a reply', [], ['~bud']],
  ]);
  assert.deepEqual((await fm.thread('test', 2)).comments, []);
  await assert.rejects(fm.thread('test', 9), /no post 9/);
  //  the base was asked once, at the desk install first, for 8 seconds
  assert.deepEqual(u.peeks.filter(p => p[1] === 'index'), [[`${DESK_BASE}/pub`, 'index', 8000]]);
  await u.close();
});

test('a host on the ball instance is found there', async () => {
  const u = await fakeUrb(board(), { indexAt: FORUM_BASES[1] });
  const b = await forum(u.urb, '~nec').board('test');
  assert.equal(b.posts.length, 2);
  assert.deepEqual(u.peeks.slice(0, 2).map(p => p[0]), FORUM_BASES.map(b => `${b}/pub`));
  await u.close();
});

test('a board not shared, or no lattice, is an UrbError', async () => {
  const u = await fakeUrb(board());
  await assert.rejects(forum(u.urb, '~nec').board('other'), e => e instanceof UrbError && /shares no board other/.test(e.message));
  await assert.rejects(forum(u.urb, '~nec').board('Bad_Name'), /not a board name/);
  await u.close();
  const v = await fakeUrb(board(), { indexAt: '/nowhere' });
  await assert.rejects(forum(v.urb, '~nec').board('test'), e => e.later === true && /no lattice peek/.test(e.message));
  await v.close();
});

test('a grub of the wrong shape fails the board, naming it', async () => {
  const bad = {
    'pub/card': [2, 'test', 'T', 'd', NEC, t0, 'poster', 0],
    'pub/roles': [1, kv([[WES, 'admin']])],
    'pub/conf': [1, [1, 2], null],
    'content/sidebar': [1, ['not', 'text']],
    'content/posts/b0': [1, kv([[1, [1, BUD, 'first', [2, 'x'], null, t0]]])],
    'content/votes/b0': [1, kv([[['reply', 1], [null, null]]])],
  };
  for (const [at, val] of Object.entries(bad)) {
    const u = await fakeUrb(board({ [at]: val }));
    await assert.rejects(forum(u.urb, '~nec').board('test'),
      e => e instanceof UrbError && e.message.includes(`unreadable: /${at}`), at);
    await u.close();
  }
  const u = await fakeUrb(Object.fromEntries(Object.entries(board()).filter(([k]) => k !== 'pub/card')));
  await assert.rejects(forum(u.urb, '~nec').board('test'), /no card/);
  await u.close();
});

test('the actions /forum-post-remote sends', async () => {
  const u = await fakeUrb(board());
  const fm = forum(u.urb, '~nec');
  await fm.newPost('test', { title: 'Hi', body: 'there' });
  await fm.newPost('test', { title: 'Link', url: 'https://x', body: '' });
  await fm.newComment('test', 1, 'a comment');
  await fm.newComment('test', 1, 'a reply', 4);
  await fm.upvote('test', { post: 1 });
  await fm.downvote('test', { post: 1, comment: 4 });
  await fm.unvote('test', { post: 1 });
  for (const p of u.pokes) assert.deepEqual(p.slice(0, 4), ['~nec', DESK_BASE, 'forum.sig', ['/lattice', 'forum-action']]);
  assert.deepEqual(u.pokes.map(p => p[4]), [
    ['new-post', 'test', 'Hi', null, [null, 'there']],
    ['new-post', 'test', 'Link', [null, 'https://x'], null],
    ['new-comment', 'test', 1, null, 'a comment'],
    ['new-comment', 'test', 1, [null, 4], 'a reply'],
    ['upvote', 'test', ['post', 1]],
    ['downvote', 'test', ['comment', 1, 4]],
    ['remove-vote', 'test', ['post', 1]],
  ]);
  assert.throws(() => actions.newPost('test', { title: '' }), /needs a title/);
  assert.throws(() => actions.newComment('test', 1, ''), /needs a body/);
  await u.close();
});

// ------------------------------------------------------------ ballOf

const leaf = nobe => ['leaf', nobe, [[null, 'noun'], 0, null], 0, null];
// a %snap and silo for a directory of { 'a/b': value }, as grubbery
// builds them, with made-up hashes (the reader checks none)
function dirSnap(files) {
  let next = 100n;
  const nouns = [], jects = [];
  const tree = prefix => {
    const names = new Set(Object.keys(files).filter(k => k.startsWith(prefix)).map(k => k.slice(prefix.length).split('/')[0]));
    const fil = [], dir = [];
    for (const n of names) {
      if (`${prefix}${n}` in files) {
        const nobe = next++, jobe = next++;
        nouns.push([nobe, [1, files[`${prefix}${n}`]]]);
        jects.push([jobe, [1, leaf(nobe)]]);
        fil.push([n, jobe]);
      } else dir.push([n, [tree(`${prefix}${n}/`), null]]);
    }
    const jobe = next++;
    jects.push([jobe, [1, ['tree', null, 0, null, kv(fil), kv(dir)]]]);
    return jobe;
  };
  const top = tree('');
  return [[[1, 0], ['firm', null, top], [null, null]], [kv(nouns), kv(jects)]];
}

test('ballOf reads a directory, deep, and refuses a loop of trees', () => {
  const [snap, silo] = dirSnap({ card: 'c', 'posts/b0': 'p0', 'posts/b1': 'p1', 'a/b/c/d': 'deep' });
  const got = decoders.ballOf(N(snap), N(silo), fs => fs.map(([p, n]) => [p.join('/'), n.text()]).sort());
  assert.deepEqual(got, [['a/b/c/d', 'deep'], ['card', 'c'], ['posts/b0', 'p0'], ['posts/b1', 'p1']]);
  //  a file's pace is not a directory's
  assert.equal(decoders.ballOf(N([[1, 0], ['tomb', null], [null, null]]), N(silo), x => x), null);
  //  a tree that holds itself is refused
  const loop = [[[1, 0], ['firm', null, 7], [null, null]],
    [null, kv([[7, [1, ['tree', null, 0, null, null, kv([['me', [7, null]]])]]]])]];
  assert.throws(() => decoders.ballOf(N(loop[0]), N(loop[1]), x => x), e => e instanceof UrbError && /holds itself/.test(e.message));
  //  and one past MAX_SEGS deep is refused
  const deep = {};
  deep[Array.from({ length: 70 }, (_, i) => `d${i}`).join('/')] = 'x';
  const [s2, l2] = dirSnap(deep);
  assert.throws(() => decoders.ballOf(N(s2), N(l2), x => x), /over 64 deep/);
});

// a silo by hand: jects {jobe: ject}, nouns {nobe: noun}, top jobe
const silo = (top, jects, nouns = {}) => [[[1, 0], ['firm', null, top], [null, null]],
  [kv(Object.entries(nouns).map(([k, v]) => [BigInt(k), [1, v]])), kv(Object.entries(jects).map(([k, v]) => [BigInt(k), [1, v]]))]];
const tree = (fil, dir = []) => ['tree', null, 0, null, kv(fil), kv(dir.map(([n, j]) => [n, [j, null]]))];
const files = s => decoders.ballOf(N(s[0]), N(s[1]), fs => fs.map(([p, n]) => [p.join('/'), n.text()]).sort());

test('ballOf: two directories alike are both read, as lattice\'s ball holds both', () => {
  //  a and b hold the same, so their trees are one ject, as hashes make it
  const got = files(silo(1, { 1: tree([['top', 2]], [['a', 4], ['b', 4]]), 2: leaf(3), 4: tree([['x', 5]]), 5: leaf(6) },
    { 3: 't', 6: 'same' }));
  assert.deepEqual(got, [['a/x', 'same'], ['b/x', 'same'], ['top', 't']]);
});

test('ballOf: hostile silos', () => {
  //  the top tree missing from the silo, or the top a leaf: nothing there
  assert.equal(files(silo(9, { 1: tree([]) })), null);
  assert.equal(files(silo(1, { 1: leaf(2) }, { 2: 'x' })), null);
  //  a file naming a tree, a missing ject or a missing noun, and a
  //  directory naming a leaf or nothing, are skipped
  assert.deepEqual(files(silo(1, {
    1: tree([['ok', 2], ['isdir', 4], ['gone', 77], ['nonoun', 5]], [['isleaf', 2], ['nodir', 88]]),
    2: leaf(3), 4: tree([]), 5: leaf(99),
  }, { 3: 'fine' })), [['ok', 'fine']]);
  //  a name over MAX_KNOT
  assert.throws(() => files(silo(1, { 1: tree([['x'.repeat(257), 2]]), 2: leaf(3) }, { 3: 'x' })), /over 256 bytes/);
  assert.throws(() => files(silo(1, { 1: tree([], [['y'.repeat(257), 4]]), 4: tree([]) })), /over 256 bytes/);
  //  a tree that is 2^20 trees by sharing, in 21 jects, is past the cap
  const jects = { 1: tree([], [['a', 2], ['b', 2]]) };
  for (let i = 2; i <= 20; i++) jects[i] = tree([], [['a', i + 1], ['b', i + 1]]);
  jects[21] = tree([]);
  assert.throws(() => files(silo(1, jects)), e => e instanceof UrbError && /over 200000 trees and files/.test(e.message));
  //  a cycle two deep
  assert.throws(() => files(silo(1, { 1: tree([], [['a', 2]]), 2: tree([], [['back', 1]]) })), /holds itself/);
});

test('a directory %snap off the wire, inline and by %want, through the engine', { timeout: 60_000 }, async () => {
  const v = await victim();
  const files = Object.fromEntries(Object.entries(board()).map(([k, x]) => [k, x]));
  const read = forum(v.urb, '~nec').board('test');
  //  answer each peek on its wire, in order: the base, /pub, /content
  //  grubbery's snap ids are (shaz eny), 512 bits
  const answer = async (n, snap, silo) => {
    await new Promise(r => setTimeout(r, 50));
    v.deliver('grubbery-transfer', [v.path(`/urb-fetch/${v.nonce}/${n}`), 'snap', [0, v.path('/x'), 'x'], 2n ** 511n + 7n, [null, snap], silo && [null, silo]]);
  };
  const index = dirSnap({ index: 0 });   // any file answers the probe: a leaf for its pace
  const indexJobe = index[1][1][0][0];   // the leaf's jobe, the first ject made
  await answer(1, [[1, 0], ['firm', null, indexJobe], [null, null]], index[1]);
  const sub = p => Object.fromEntries(Object.entries(files).filter(([k]) => k.startsWith(p + '/')).map(([k, x]) => [k.slice(p.length + 1), x]));
  await answer(2, ...dirSnap(sub('pub')));
  //  /content by the %want leg, as grubbery answers past 8 KB: the snap
  //  comes first with no data, then the silo on the fixed wire /want
  const [snap, silo] = dirSnap(sub('content'));
  await answer(3, snap, null);
  await new Promise(r => setTimeout(r, 50));
  v.deliver('grubbery-transfer', [v.path('/want'), 'data', silo]);
  const b = await late(read);
  assert.equal(b.info.title, 'Test board');
  assert.deepEqual(b.posts.map(p => p.id), [2, 1]);
  assert.equal(b.comments[1].length, 2);
  assert.deepEqual(v.trapped(), []);
  await v.f.close();
});

test('a %veto of /pub, or of /content after /pub, fails the board cleanly', { timeout: 60_000 }, async () => {
  for (const vetoed of [2, 3]) {
    const v = await victim();
    const read = forum(v.urb, '~nec').board('test').then(() => null, e => e);
    const say = async (n, rest) => {
      await new Promise(r => setTimeout(r, 50));
      v.deliver('grubbery-transfer', [v.path(`/urb-fetch/${v.nonce}/${n}`), ...rest]);
    };
    const index = dirSnap({ index: 0 });
    await say(1, ['snap', [0, v.path('/x'), 'x'], 1, [null, [[1, 0], ['firm', null, index[1][1][0][0]], [null, null]]], [null, index[1]]]);
    const pub = Object.fromEntries(Object.entries(board()).filter(([k]) => k.startsWith('pub/')).map(([k, x]) => [k.slice(4), x]));
    if (vetoed === 3) await say(2, ['snap', [1, v.path('/x')], 1, ...((s => [[null, s[0]], [null, s[1]]])(dirSnap(pub)))]);
    await say(vetoed, ['veto', [1, v.path('/x')]]);
    try {
      const e = await late(read);
      assert.ok(e instanceof UrbError && /shares no board test/.test(e.message), `${vetoed}: ${e}`);
      assert.deepEqual(v.trapped(), []);
    } finally {
      await v.f.close();
    }
  }
});

test('dates past a Date are null, and ids past 2^53 are exact', async () => {
  const far = (2n ** 255n);
  const u = await fakeUrb(board({
    'content/posts/b0': [1, kv([[2n ** 60n + 1n, [2n ** 60n + 1n, BUD, 'big', null, null, far]], [2n ** 60n, [2n ** 60n, BUD, 'big too', null, null, t0]]])],
    'pub/conf': [1, 2n ** 60n + 2n, null],
  }));
  const b = await forum(u.urb, '~nec').board('test');
  assert.deepEqual(b.posts.map(p => [p.id, p.created]), [[2n ** 60n + 1n, null], [2n ** 60n, new Date(Date.UTC(2026, 9, 31, 12))]]);
  assert.equal(b.nextPost, 2n ** 60n + 2n);
  assert.equal((await forum(u.urb, '~nec').thread('test', '1152921504606846976')).post.title, 'big too');
  await u.close();
  assert.equal(idOf(5), 5);
  assert.equal(idOf(5n), 5);
  assert.equal(idOf('9007199254740993'), 9007199254740993n);
  for (const bad of [-1, 1.5, NaN, '01', '-1', 2 ** 53, -1n, 2n ** 256n, null, {}]) assert.throws(() => idOf(bad), UrbError, String(bad));
  assert.throws(() => actions.upvote('test', { post: -1 }), /not an id/);
  assert.throws(() => actions.newComment('test', 1.5, 'x'), /not an id/);
  assert.throws(() => actions.newComment('test', 1, 'x', -2), /not an id/);
  assert.deepEqual(actions.downvote('test', { post: 2n ** 60n, comment: '3' }), ['downvote', 'test', ['comment', 2n ** 60n, 3]]);
});

// ------------------------------------------------------------ two engines

test('a post reaches the host\'s /forum.sig, and its refusal note comes back', { timeout: 60_000 }, async () => {
  const f = fleet();
  const zod = await f.engine('~zod', 0), nec = await f.engine('~nec', 1);
  //  ~nec runs lattice at the desk install: its /pub/index answers
  nec.grub.serve((folder, name) => (folder === `${DESK_BASE}/pub` && name === 'index'
    ? { mark: ['/', 'noun'], value: 0, rev: 1, at: Date.now() } : null));
  const heard = [];
  nec.grub.onInbox(({ from, to, mark, noun }) => {
    const act = [noun.head.text(), noun.tail.head.text(), noun.tail.tail.head.text()];
    heard.push({ from, to, mark, act });
  });
  const fm = forum(zod, '~nec');
  const notes = [];
  fm.onNote(n => notes.push(n));
  const r = await late(fm.newPost('test', { title: 'Hello', body: 'From a comet' }));
  assert.deepEqual(r, { ok: true });
  assert.deepEqual(heard, [{ from: '~zod', to: [DESK_BASE, 'forum.sig'], mark: ['/lattice', 'forum-action'], act: ['new-post', 'test', 'Hello'] }]);
  //  the host refuses it, as +forum-note-send does
  const sent = await late(nec.grub.poke('~zod', DESK_BASE, 'forum.sig', ['/lattice', 'forum-note'], ['test', 'posting too fast']));
  assert.deepEqual(sent, { ok: true });
  assert.equal(notes.length, 1);
  assert.deepEqual({ ...notes[0], at: undefined }, { host: '~nec', board: 'test', reason: 'posting too fast', at: undefined });
  assert.ok(notes[0].at instanceof Date);
  //  a note of the wrong shape, or another mark, is not one
  await late(nec.grub.poke('~zod', DESK_BASE, 'forum.sig', ['/lattice', 'forum-note'], [['a', 'b'], 'c']));
  await late(nec.grub.poke('~zod', DESK_BASE, 'forum.sig', ['/lattice', 'forum-action'], ['test', 'x']));
  //  nor is one to another grub of ours
  await late(nec.grub.poke('~zod', DESK_BASE, 'inbox.sig', ['/lattice', 'forum-note'], ['test', 'elsewhere']));
  //  and a note from a ship that isn't the host is not its: anyone could
  //  send one, and a refusal stops the poster's wait
  const bud = await f.engine('~bud', 2);
  await late(bud.grub.poke('~zod', DESK_BASE, 'forum.sig', ['/lattice', 'forum-note'], ['test', 'a fake refusal']));
  assert.equal(notes.length, 1);
  await f.close();
});
