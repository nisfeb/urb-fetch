// A lattice forum board on another ship, from Node (forum.mjs on mesa.mjs):
//
//   node forum-cli.mjs [node.mjs's options] urb://~host/forum/board
//       the board's posts
//   node forum-cli.mjs [node.mjs's options] urb://~host/forum/board/ID
//       a post and its comments
//   ... urb://~host/forum/board --post TITLE [--body TEXT] [--url URL]
//       posts, then reads the board until the post is there, or the host
//       refuses it with a note
//   ... urb://~host/forum/board/ID --comment TEXT    comments, and reads it back
//   ... urb://~host/forum/board/ID --upvote          votes, and reads it back
//
// On a fake network, for lattice on furum's fleet set:
//   node forum-cli.mjs --fake ~sampel-sampel-dozzod-dozhus urb://~hus/forum/m1btest

import { parseArgs } from 'node:util';
import { forum } from './forum.mjs';
import { deUrb } from './mesa.mjs';
import { connect, log, options } from './node.mjs';

const { values: opt, positionals: [url] } = parseArgs({
  allowPositionals: true,
  options: {
    ...options, post: { type: 'string' }, body: { type: 'string' }, url: { type: 'string' },
    comment: { type: 'string' }, upvote: { type: 'boolean' },
  },
});
const [host, [name, id] = [], mount] = url ? deUrb(url) : [];
if (mount !== 'forum' || ((opt.comment || opt.upvote) && id === undefined)) {
  log('usage: node forum-cli.mjs [--fake ~SHIP | --relay URL] [--moon KEYFILE] [--trace] urb://~host/forum/board[/ID] [--post TITLE [--body TEXT] [--url URL] | --comment TEXT | --upvote]');
  process.exit(2);
}

// poll f until it gives something, for up to a minute, or the host refuses
let refused = null;
async function until(f) {
  for (let i = 0; i < 30; i++) {
    if (refused) return null;
    const v = await f();
    if (v) return v;
    await new Promise(r => setTimeout(r, 2000));
  }
  return null;
}
const votes = x => `${x.up.length} up, ${x.down.length} down`;
const show = p => `#${p.id} ${p.title}  (${p.author}, ${p.created.toISOString().slice(0, 16)}, ${votes(p)}, ${p.comments} comments)${p.url ? `\n    ${p.url}` : ''}${p.body ? `\n    ${p.body}` : ''}`;
const showThread = t => {
  console.log(show(t.post));
  for (const c of t.comments) console.log(`  c${c.id}${c.parent !== null ? ` on c${c.parent}` : ''} ${c.author}: ${c.body} (${votes(c)})`);
};
let code = 0, urb = null;
try {
  urb = await connect(opt);
  const f = forum(urb, host);
  f.onNote(n => {
    log(`note from ${n.host} on ${n.board}: ${n.reason}`);
    refused = n.reason;
  });
  let t = Date.now();
  const before = await f.board(name);
  log(`${before.info.title} (${host}/${name}, ${before.info.public ? 'public' : 'private'}): ${before.posts.length} posts, read in ${((Date.now() - t) / 1000).toFixed(1)}s`);
  const done = (sent, what, there) => {
    log(`${what}: ${sent.ok ? '/forum.sig took it' : `not taken: ${sent.error}`} (${((Date.now() - t) / 1000).toFixed(1)}s)`);
    if (!sent.ok) throw new Error(sent.error);
    return until(there).then(v => {
      if (refused) throw new Error(`refused: ${refused}`);
      if (!v) throw new Error('it never appeared on the board');
      log(`on the board after ${((Date.now() - t) / 1000).toFixed(1)}s`);
      return v;
    });
  };
  if (opt.post) {
    t = Date.now();
    const sent = await f.newPost(name, { title: opt.post, body: opt.body, url: opt.url });
    console.log(show(await done(sent, 'posted', async () =>
      (await f.board(name)).posts.find(p => p.title === opt.post && p.author === urb.our && !before.posts.some(b => b.id === p.id)))));
  } else if (opt.comment || opt.upvote) {
    t = Date.now();
    const sent = opt.comment ? await f.newComment(name, id, opt.comment) : await f.upvote(name, { post: id });
    await done(sent, opt.comment ? 'commented' : 'voted', async () => {
      const th = await f.thread(name, id);
      return (opt.comment ? th.comments.some(c => c.author === urb.our && c.body === opt.comment) : th.post.up.includes(urb.our)) && th;
    });
    showThread(await f.thread(name, id));
  } else if (id !== undefined) {
    showThread(await f.thread(name, id));
  } else {
    for (const p of before.posts) console.log(show(p));
  }
} catch (e) {
  log(e.message || e);
  code = 1;
} finally {
  await urb?.close();
}
process.exit(code);
