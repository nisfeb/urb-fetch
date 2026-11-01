// forum.mjs on the Rust core (docs/rust-core.md, M6): lattice's forum
// boards on another ship, over urb://, read and written by the core in
// mesa.wasm (core/urb/src/core/apps.rs, forum.rs), with wasm/forum.mjs's
// interface: board, thread, newPost, newComment, upvote, downvote, unvote
// and onNote, giving what they gave.

import { CORE, DESK_BASE } from './mesa.mjs';

export const FORUM_BASES = [DESK_BASE, '/apps/lattice.lattice_app'];   // +peer-bases

export function forum(urb, host) {
  const c = urb[CORE];
  const call = (method, ...a) => c.call('forum', host, method, ...a);
  return {
    board: name => call('board', name),
    thread: (name, post) => call('thread', name, post),
    newPost: (name, post) => call('newPost', name, post),
    newComment: (name, post, body, parent = null) => call('newComment', name, post, body, parent),
    upvote: (name, target) => call('upvote', name, target),
    downvote: (name, target) => call('downvote', name, target),
    unvote: (name, target) => call('unvote', name, target),
    // f({host, board, reason, at}) for each refusal note this host sends
    // us. It takes the engine's one inbox, as furum's onNote does.
    onNote(f) {
      c.on['forum.note'] = f;
      c.callSync('forum', host, 'onNote');
    },
  };
}
