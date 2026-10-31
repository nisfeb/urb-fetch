// lattice's forum boards on another ship, for a mesa.mjs engine: read a
// board and a thread, post, comment and vote, and hear why the host
// refused one, as lattice release 81 does it (github.com/nisfeb/lattice,
// code/nex/lattice/app.hoon, code/lib/lattice-forum-*.hoon):
//
//   the host's base   +peer-base: <base>/pub/index answers at the desk
//                     install, else at the ball instance, each asked for
//                     at most 8 seconds
//   a board           +forum-read-grubs-remote: two deep peeks, of
//                     <base>/forum/boards/<board>/pub and /content, the
//                     two directories a public board grants (and a
//                     private one grants its members). load:fb reads
//                     the grubs:
//     pub/card               [%1 board-info]
//     pub/roles              [%1 (map @p role)]
//     pub/conf               [%1 next-post=@ud prune=(unit [min-score=@ud after=@dr])]
//     content/pins           [%1 (set post-id)]
//     content/sidebar        [%1 @t]
//     content/posts/b<k>     [%1 (map post-id post-core)], k = (id - 1) / 100
//     content/threads/b<k>   [%1 (map post-id [next (map comment-id comment-core)])]
//     content/votes/b<k>     [%1 (map vote-target [up=(set @p) down=(set @p)])]
//   an action         poked at the host's <base>/forum.sig as
//                     [/lattice %forum-action] action:ft. The host stamps
//                     us as its author from the transport. Only what
//                     lattice's /forum-post-remote sends: a post, a
//                     comment, an upvote, a downvote, an unvote.
//   a refusal         the host pokes our <base>/forum.sig with
//                     [/lattice %forum-note] [board=@tas reason=@t], and
//                     lattice keeps it as [host board reason at], with
//                     host the sender and at when it came
//
// The host is another ship, so what it sends is hostile: mesa.mjs reads
// every noun under its caps, and a grub of the wrong shape fails the read
// with an UrbError naming it ('unreadable: /pub/conf'), as load:fb
// refuses a board it can read only part of. Unlike lattice's clam, the
// treap order of a map or set is not checked.

import { DESK_BASE, UrbError } from './mesa.mjs';

export const FORUM_BASES = [DESK_BASE, '/apps/lattice.lattice_app'];   // +peer-bases
const BASE_WAIT = 8_000;
const DA_UNIX_EPOCH = 0x8000000cce9e0d80n;
const ACTION = ['/lattice', 'forum-action'];
const NOTE = ['/lattice', 'forum-note'];
const ROLES = ['mod', 'poster', 'reader'];
const TAS = /^[a-z][a-z0-9-]*$/;   // (sane %tas), which a board name must be
const SAFE = BigInt(Number.MAX_SAFE_INTEGER);
const MAX_MS = 8_640_000_000_000_000n;   // the furthest a Date reaches either way

// An id or count, exactly: a number up to 2^53 - 1, a BigInt past it, so
// each value has one form and === compares ids. ids sorts by them.
export const exact = v => (v <= SAFE ? Number(v) : v);
export const ids = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
// an id we are given: a non-negative integer as a number, BigInt or
// digits, in its exact form
export function idOf(x) {
  let v = null;
  if (typeof x === 'bigint') v = x;
  else if (typeof x === 'number' && Number.isSafeInteger(x)) v = BigInt(x);
  else if (typeof x === 'string' && /^(0|[1-9][0-9]*)$/.test(x) && x.length <= 77) v = BigInt(x);
  if (v === null || v < 0n || v >= 2n ** 256n) throw new UrbError(`not an id: ${x}`);
  return exact(v);
}

// The forum actions lattice's /forum-post-remote sends, as nouns for
// mesa.mjs (action:ft in code/lib/lattice-forum-types.hoon). As its form
// does, an empty url, body or parent is none, and a post with no title
// or a comment with no body is refused before it is sent. target is
// {post} or {post, comment}.
const some = v => (v === null || v === undefined || v === '' ? null : [null, v]);
const targetOf = ({ post, comment = null }) => (comment === null ? ['post', idOf(post)] : ['comment', idOf(post), idOf(comment)]);
export const actions = {
  newPost(name, { title, url = null, body = null }) {
    if (!title) throw new UrbError('a post needs a title');
    return ['new-post', name, title, some(url), some(body)];
  },
  newComment(name, post, body, parent = null) {
    if (!body) throw new UrbError('a comment needs a body');
    return ['new-comment', name, idOf(post), parent === null ? null : [null, idOf(parent)], body];
  },
  upvote: (name, target) => ['upvote', name, targetOf(target)],
  downvote: (name, target) => ['downvote', name, targetOf(target)],
  unvote: (name, target) => ['remove-vote', name, targetOf(target)],
};

export function forum(urb, host) {
  const { entries, ship } = urb.nouns;
  let base = null;   // the host's base, once it answered

  //  shapes, each refusing what lattice's clam would
  const atom = n => {
    if (n.isCell) throw new UrbError('a cell where an atom goes');
    return n;
  };
  const big = n => atom(n).int();
  const num = n => exact(big(n));
  const text = n => atom(n).text();
  const flag = n => {
    const v = big(n);
    if (v > 1n) throw new UrbError('not a flag');
    return v === 0n;
  };
  const unit = (n, f) => {
    if (!n.isCell) {
      if (big(n) !== 0n) throw new UrbError('not a unit');
      return null;
    }
    if (big(n.head) !== 0n) throw new UrbError('not a unit');
    return f(n.tail);
  };
  //  a @da as a Date, or null for one past where a Date reaches
  const date = n => {
    const d = big(n);
    const ms = ((d >> 64n) - DA_UNIX_EPOCH) * 1000n + (((d & 0xffffffffffffffffn) * 1000n) >> 64n);
    return ms > MAX_MS || ms < -MAX_MS ? null : new Date(Number(ms));
  };
  const role = n => {
    const r = text(n);
    if (!ROLES.includes(r)) throw new UrbError(`no role ${r}`);
    return r;
  };
  const ships = n => entries(n).map(s => ship(atom(s)));
  const tuple = (n, k) => {   // the k items of an n-tuple
    const out = [];
    for (let i = 0; i < k - 1; i++, n = n.tail) out.push(n.head);
    return [...out, n];
  };
  const versioned = n => {    // [%1 x]
    if (big(n.head) !== 1n) throw new UrbError('not version 1');
    return n.tail;
  };
  const info = n => {
    const [name, title, description, h, created, defaultRole, pub] = tuple(n, 7);
    return { name: text(name), title: text(title), description: text(description), host: ship(atom(h)),
             created: date(created), defaultRole: role(defaultRole), public: flag(pub) };
  };
  const postCore = n => {
    const [id, author, title, url, body, created] = tuple(n, 6);
    return { id: num(id), author: ship(atom(author)), title: text(title), url: unit(url, text),
             body: unit(body, text), created: date(created) };
  };
  const commentCore = n => {
    const [id, parent, author, body, created] = tuple(n, 5);
    return { id: num(id), parent: unit(parent, num), author: ship(atom(author)), body: text(body), created: date(created) };
  };
  const target = n => {
    const tag = text(n.head);
    if (tag === 'post') return `post ${num(n.tail)}`;
    if (tag === 'comment') return `comment ${num(n.tail.head)} ${num(n.tail.tail)}`;
    throw new UrbError(`no vote target %${tag}`);
  };
  const map = (n, k, v) => entries(n).map(e => [k(e.head), v(e.tail)]);

  // load:fb, in two steps, since a grub's noun is borrowed only during
  // its peek: read(acc, pax, noun) reads one grub into acc, during the
  // peek, and fails with an UrbError naming the first it can't read; then
  // assemble(acc) is the board. A grub it doesn't know is skipped.
  const fresh = () => ({ card: null, roles: [], conf: { nextPost: 0, prune: null }, pins: [], side: '',
                         cores: new Map(), threads: new Map(), votes: new Map() });
  function read(acc, pax, val) {
    const at = pax.join('/');
    try {
      if (at === 'pub/card') acc.card = info(versioned(val));
      else if (at === 'pub/roles') acc.roles = map(versioned(val), n => ship(atom(n)), role);
      else if (at === 'pub/conf') {
        const c = versioned(val);
        acc.conf = { nextPost: num(c.head), prune: unit(c.tail, p => ({ minScore: num(p.head),
          after: exact((big(p.tail) * 1000n) >> 64n) })) };   // ms
      } else if (at === 'content/pins') acc.pins = entries(versioned(val)).map(num);
      else if (at === 'content/sidebar') acc.side = text(versioned(val));
      else if (pax.length === 3 && pax[0] === 'content') {
        const v = versioned(val);
        if (pax[1] === 'posts') for (const [id, p] of map(v, num, postCore)) acc.cores.set(id, p);
        else if (pax[1] === 'threads') {
          for (const [id, t] of map(v, num, t => ({ next: num(t.head), comments: map(t.tail, num, commentCore) }))) acc.threads.set(id, t);
        } else if (pax[1] === 'votes') for (const [k, t] of map(v, target, t => ({ up: ships(t.head), down: ships(t.tail) }))) acc.votes.set(k, t);
      }
    } catch (e) {
      if (!(e instanceof UrbError)) throw e;
      throw new UrbError(`${host}'s board grub is unreadable: /${at} (${e.message})`);
    }
  }

  function assemble(acc) {
    if (!acc.card) throw new UrbError(`${host}'s board has no card`);
    const tally = k => acc.votes.get(k) ?? { up: [], down: [] };
    const posts = [...acc.cores.values()].map(c => {
      const { up, down } = tally(`post ${c.id}`);
      return { ...c, up, down, comments: acc.threads.get(c.id)?.comments.length ?? 0 };
    });
    const comments = {};
    for (const [pid, t] of acc.threads) {
      comments[pid] = t.comments.map(([cid, c]) => ({ ...c, ...tally(`comment ${pid} ${cid}`) })).sort((a, b) => ids(a.id, b.id));
    }
    return {
      info: acc.card,
      roles: Object.fromEntries(acc.roles),
      nextPost: acc.conf.nextPost,
      prune: acc.conf.prune,
      pinned: [...acc.pins].sort(ids),
      sidebar: acc.side,
      posts: posts.sort((a, b) => ids(b.id, a.id)),
      comments,
    };
  }

  // +peer-base-cached, cached here for this reader's life
  async function hostBase() {
    if (base) return base;
    for (const b of FORUM_BASES) {
      if (await urb.grub.peek(host, `${b}/pub`, 'index', () => true, BASE_WAIT)) return (base = b);
    }
    throw Object.assign(new UrbError(`${host} answered no lattice peek (offline, or no lattice)`), { later: true });
  }

  const boardName = name => {
    if (typeof name !== 'string' || !TAS.test(name)) throw new UrbError(`not a board name: ${name}`);
    return name;
  };

  // the board's two directories read into one acc, or null if either is
  // not there. One at a time, as lattice peeks them
  async function grubs(name) {
    const dir = `${await hostBase()}/forum/boards/${boardName(name)}`, acc = fresh();
    for (const sub of ['pub', 'content']) {
      const got = await urb.grub.peek(host, `${dir}/${sub}`, null, files => {
        for (const [pax, n] of files) read(acc, [sub, ...pax], n);
        return true;
      });
      if (!got) return null;
    }
    return acc;
  }

  async function board(name) {
    const acc = await grubs(name);
    if (!acc) throw new UrbError(`${host} shares no board ${name} with ${urb.our}: unreachable, private or not there`);
    return assemble(acc);
  }

  const act = action => hostBase().then(b => urb.grub.poke(host, b, 'forum.sig', ACTION, action));

  return {
    // the whole board: {info, roles, nextPost, prune, pinned, sidebar,
    // posts (newest first, each with up, down and its comment count),
    // comments ({[post]: oldest first})}. Ids and counts are exact (see
    // exact), and a date a Date can't hold is null.
    board,

    // one post and its comments, oldest first
    async thread(name, post) {
      post = idOf(post);
      const b = await board(name);
      const p = b.posts.find(x => x.id === post);
      if (!p) throw new UrbError(`no post ${post} on ${host}'s board ${name}`);
      return { info: b.info, post: p, comments: b.comments[post] ?? [] };
    },

    // {ok} once the host's /forum.sig took it in; the host then applies it,
    // or refuses it with a note
    newPost: (name, post) => act(actions.newPost(boardName(name), post)),
    newComment: (name, post, body, parent = null) => act(actions.newComment(boardName(name), post, body, parent)),
    upvote: (name, target) => act(actions.upvote(boardName(name), target)),
    downvote: (name, target) => act(actions.downvote(boardName(name), target)),
    unvote: (name, target) => act(actions.unvote(boardName(name), target)),

    // f({host, board, reason, at}) for each refusal note this host sends
    // us, to our /forum.sig: a note from any other ship is not its, and is
    // ignored. It takes the engine's one inbox, as furum's onNote does.
    onNote(f) {
      urb.grub.onInbox(({ from, to, mark, noun }) => {
        if (from !== host || to[1] !== 'forum.sig' || !FORUM_BASES.includes(to[0])) return;
        if (mark[0] !== NOTE[0] || mark[1] !== NOTE[1] || !noun.isCell || noun.head.isCell || noun.tail.isCell) return;
        f({ host: from, board: noun.head.text(), reason: noun.tail.text(), at: new Date(urb.now()) });
      });
    },
  };
}
