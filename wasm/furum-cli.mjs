// A furum board on another ship, from Node (furum.mjs on mesa.mjs):
//
//   node furum-cli.mjs [node.mjs's options] ~host board
//       lists the board's posts
//   node furum-cli.mjs [node.mjs's options] ~host board --post TITLE [--body TEXT] [--url URL]
//       posts, then reads the board until the post is there
//   ... ~host board --thread ID              a post's comments and votes
//   ... ~host board --comment ID --body TEXT comments, and reads it back
//   ... ~host board --upvote ID              votes, and reads it back
//
// On a fake network, for furum's fleet set:
//   node furum-cli.mjs --fake ~doznec-dozzod-dozhus ~rup urbtest

import { parseArgs } from 'node:util';
import { furum } from './furum.mjs';
import { connect, log, options } from './node.mjs';

const { values: opt, positionals: [host, name] } = parseArgs({
  allowPositionals: true,
  options: {
    ...options, post: { type: 'string' }, body: { type: 'string' }, url: { type: 'string' },
    thread: { type: 'string' }, comment: { type: 'string' }, upvote: { type: 'string' },
  },
});
if (!host || !name) {
  log('usage: node furum-cli.mjs [--fake ~SHIP | --relay URL] [--moon KEYFILE] [--trace] ~host board [--post TITLE [--body TEXT] [--url URL]]');
  process.exit(2);
}

// poll f until it gives something, for up to a minute, or furum refuses
let refused = null;
async function until(f) {
  for (let i = 0; i < 30; i++) {
    if (refused) throw new Error(`refused: ${refused}`);
    const v = await f();
    if (v) return v;
    await new Promise(r => setTimeout(r, 2000));
  }
  return null;
}
const showThread = async id => {
  const [cs, vs] = await Promise.all([f.comments(name, id), f.votes(name, id)]);
  console.log(`#${id}: ${vs.up.length} up${vs.up.length ? ` (${vs.up.join(' ')})` : ''}, ${vs.down.length} down, ${cs.length} comments`);
  for (const c of cs) console.log(`  c${c.id}${c.parent !== null ? ` on c${c.parent}` : ''} ${c.author}: ${c.body}`);
};
let f;
const show = p => `#${p.id} ${p.title}  (${p.author}, ${p.created.toISOString().slice(0, 16)})${p.url ? `\n    ${p.url}` : ''}${p.body ? `\n    ${p.body}` : ''}`;
let code = 0, urb = null;
try {
  urb = await connect(opt);
  f = furum(urb, host);
  f.onNote(n => {
    log(`note from ${n.from}: ${n.title}: ${n.body}`);
    refused = n.body;
  });
  let t = Date.now();
  const before = await f.posts(name);
  log(`${before.length} posts on ${host}/${name}, read in ${((Date.now() - t) / 1000).toFixed(1)}s`);
  if (opt.thread) {
    await showThread(Number(opt.thread));
  } else if (opt.comment || opt.upvote) {
    const id = Number(opt.comment || opt.upvote);
    t = Date.now();
    const sent = opt.comment ? await f.newComment(name, id, opt.body || '') : await f.upvote(name, id);
    log(`${opt.comment ? 'commented' : 'voted'}: ${sent.ok ? 'the inbox took it' : `refused: ${sent.error}`} (${((Date.now() - t) / 1000).toFixed(1)}s)`);
    if (!sent.ok) throw new Error(sent.error);
    const there = await until(async () => (opt.comment
      ? (await f.comments(name, id)).find(c => c.author === urb.our && c.body === opt.body)
      : (await f.votes(name, id)).up.includes(urb.our)));
    if (!there) throw new Error('it never appeared on the board');
    log(`on the board after ${((Date.now() - t) / 1000).toFixed(1)}s`);
    await showThread(id);
  } else if (!opt.post) {
    for (const p of before) console.log(show(p));
  } else {
    t = Date.now();
    const sent = await f.newPost(name, { title: opt.post, body: opt.body, url: opt.url });
    log(`posted: ${sent.ok ? 'the inbox took it' : `refused: ${sent.error}`} (${((Date.now() - t) / 1000).toFixed(1)}s)`);
    if (!sent.ok) throw new Error(sent.error);
    const mine = await until(async () =>
      (await f.posts(name)).find(p => p.title === opt.post && p.author === urb.our && !before.some(b => b.id === p.id)));
    if (!mine) throw new Error('the post never appeared on the board');
    log(`on the board after ${((Date.now() - t) / 1000).toFixed(1)}s`);
    console.log(show(mine));
  }
} catch (e) {
  log(e.message || e);
  code = 1;
} finally {
  await urb?.close();
}
process.exit(code);
