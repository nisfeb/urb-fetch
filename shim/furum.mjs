// furum.mjs on the Rust core (docs/rust-core.md, M6): furum's boards on
// another ship, read and written by the core in mesa.wasm
// (core/urb/src/core/apps.rs, forum.rs), with wasm/furum.mjs's interface:
// nextPost, posts, comments, votes, newPost, newComment, upvote and
// onNote, giving what they gave.

import { CORE } from './mesa.mjs';

export const MAX_POSTS = 100_000;
export const FURUM_BASE = '/apps/shell.shell/desks/furum.desk/desk/data/furum.furum_app';

export function furum(urb, host, base = FURUM_BASE) {
  if (base !== FURUM_BASE) throw new TypeError(`furum: the Rust core reads furum at ${FURUM_BASE} only`);
  const c = urb[CORE];
  const call = (method, ...a) => c.call('furum', host, method, ...a);
  return {
    nextPost: name => call('nextPost', name),
    posts: name => call('posts', name),
    comments: (name, postId) => call('comments', name, postId),
    votes: (name, postId) => call('votes', name, postId),
    newPost: (name, post) => call('newPost', name, post),
    newComment: (name, postId, body, parent = null) => call('newComment', name, postId, body, parent),
    upvote: (name, postId) => call('upvote', name, postId),
    // f({from, title, body}) for each note furum sends us, such as a
    // refusal. It takes the engine's one inbox.
    onNote(f) {
      c.on['furum.note'] = f;
      c.callSync('furum', host, 'onNote');
    },
  };
}
