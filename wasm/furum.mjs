// furum's boards on another ship, for a mesa.mjs engine: read a board's
// posts, and post, comment and vote, as whoever the engine runs as. The
// shapes are furum's own (github.com/nisfeb/furum, release 7):
//
//   <base>/boards/<name>/pub/conf          [%1 next-post=@ud prune]
//   <base>/boards/<name>/content/posts/b<k> [%1 (map post-id post-core)],
//       post-core [id=@ud author=@p title=@t url=(unit @t) body=(unit @t)
//       created=@da], k = id / 100 (code/lib/furum-board.hoon)
//   <base>/boards/<name>/content/threads/b<k> [%1 (map post-id thread)],
//       thread [next comments=(map comment-id comment-core)], comment-core
//       [id parent=(unit comment-id) author=@p body=@t created=@da]
//   <base>/boards/<name>/content/votes/b<k> [%1 (map vote-target tally)],
//       vote-target [%post id] or [%comment post id], tally [up down]
//   <base>/inbox.sig takes [/furum %msg] [%act action] from any ship
//       (code/nex/furum/app.hoon, code/lib/furum-types.hoon)
//
// Every ship may read a board's pub/ and a free board's content/. An
// action that furum refuses comes back as a [%note title body ...] poked
// into our own inbox.sig, which onNote hears.
//
// The host is another ship, so what it sends is hostile: mesa.mjs reads
// every noun under its caps (MAX_CELLS and the rest), and a record of the
// wrong shape fails the read with an UrbError. A board may claim at most
// MAX_POSTS posts, 1,000 buckets read one peek each; past that, posts()
// refuses it, rather than peek for ever.

import { UrbError } from './mesa.mjs';

export const MAX_POSTS = 100_000;
export const FURUM_BASE = '/apps/shell.shell/desks/furum.desk/desk/data/furum.furum_app';
const DA_UNIX_EPOCH = 0x8000000cce9e0d80n;
const MSG = ['/furum', 'msg'];

export function furum(urb, host, base = FURUM_BASE) {
  const { entries, unit, ship } = urb.nouns;
  const board = name => `${base}/boards/${name}`;
  const text = n => (n ? n.text() : null);
  const date = n => {
    const d = n.int();
    return new Date(Number((d >> 64n) - DA_UNIX_EPOCH) * 1000 + Number(((d & 0xffffffffffffffffn) * 1000n) >> 64n));
  };
  const post = p => {
    const [id, author, title, url, body, created] = [p.head, p.tail.head, p.tail.tail.head, p.tail.tail.tail.head,
      p.tail.tail.tail.tail.head, p.tail.tail.tail.tail.tail];
    return { id: Number(id.int()), author: ship(author), title: title.text(), url: text(unit(url)), body: text(unit(body)), created: date(created) };
  };
  const comment = c => {
    const [id, parent, author, body, created] = [c.head, c.tail.head, c.tail.tail.head, c.tail.tail.tail.head, c.tail.tail.tail.tail];
    const up = unit(parent);
    return { id: Number(id.int()), parent: up ? Number(up.int()) : null, author: ship(author), body: body.text(), created: date(created) };
  };
  //  the entry for one id in a bucket of [%1 (map @ud value)]
  const inBucket = (folder, id, conv) => urb.grub.peek(host, folder, `b${Math.floor(id / 100)}`,
    n => { const e = entries(n.tail).find(e => Number(e.head.int()) === id); return e ? conv(e.tail) : undefined; });
  const act = (...action) => urb.grub.poke(host, base, 'inbox.sig', MSG, ['act', ...action]);
  const some = v => (v ? [null, v] : null);

  return {
    // the id the board's next post gets, or null: no such board, or not shared
    nextPost: name => urb.grub.peek(host, `${board(name)}/pub`, 'conf', n => (n.isCell ? Number(n.tail.head.int()) : null)),

    // the board's posts, newest first
    async posts(name) {
      const next = await this.nextPost(name);
      if (next === null) throw new Error(`no board ${name} on ${host}, or it is not shared`);
      if (next > MAX_POSTS) throw new UrbError(`${host}'s board ${name} claims ${next} posts, over ${MAX_POSTS}`);
      const out = [];
      for (let k = 0; k * 100 < next; k++) {
        const got = await urb.grub.peek(host, `${board(name)}/content/posts`, `b${k}`, n => entries(n.tail).map(e => post(e.tail)));
        if (got) out.push(...got);   //  an empty bucket isn't there
      }
      return out.sort((a, b) => b.id - a.id);
    },

    // a post's comments, oldest first ([] before the first)
    async comments(name, postId) {
      const got = await inBucket(`${board(name)}/content/threads`, postId, t => entries(t.tail).map(e => comment(e.tail)));
      return (got || []).sort((a, b) => a.id - b.id);
    },

    // a post's votes: {up, down}, lists of ships
    async votes(name, postId) {
      const got = await urb.grub.peek(host, `${board(name)}/content/votes`, `b${Math.floor(postId / 100)}`, n => {
        const e = entries(n.tail).find(e => e.head.head.text() === 'post' && Number(e.head.tail.int()) === postId);
        return e ? { up: entries(e.tail.head).map(ship), down: entries(e.tail.tail).map(ship) } : null;
      });
      return got || { up: [], down: [] };
    },

    // {ok} once the host's inbox took it in; furum then applies it, or
    // refuses it with a note
    newPost: (name, { title, url = null, body = null }) => act('new-post', name, title, some(url), some(body)),
    newComment: (name, postId, body, parent = null) => act('new-comment', name, postId, some(parent), body),
    upvote: (name, postId) => act('upvote', name, ['post', postId]),

    // f({from, title, body}) for each note furum sends us, such as a refusal
    onNote(f) {
      urb.grub.onInbox(({ from, mark, noun }) => {
        if (mark[0] !== MSG[0] || mark[1] !== MSG[1] || noun.head.text() !== 'note') return;
        f({ from, title: noun.tail.head.text(), body: noun.tail.tail.head.text() });
      });
    },
  };
}
