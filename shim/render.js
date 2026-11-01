//  render.js: render.wasm (core/render-wasm), urb-render's extension
//  half and mesa.mjs's pure helpers, for the shims whose calls are
//  synchronous: gemtext.js, markdown.js, and mesa.mjs's renderAs,
//  looksHtml, deUrb, uv and da (docs/rust-core.md, M6).
//
//  The extension's service worker imports gemtext.js and mesa.mjs, and a
//  module service worker may not use a top-level await, so the module is
//  compiled synchronously, from the bytes render-wasm.js carries (built
//  by shim/build.sh). It is about 280 KB, under the 8 MB a browser lets a
//  page compile synchronously on its main thread.

import RENDER_WASM from './render-wasm.js'
import { wasi } from './wasi.js'

const bytes = Uint8Array.from(atob(RENDER_WASM), (c) => c.charCodeAt(0))
let X = null
X = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
  wasi_snapshot_preview1: wasi(() => X.memory, console.error, {}, 'render.wasm'),
}).exports

const enc = new TextEncoder(), dec = new TextDecoder()

//  fn(...args) in render.wasm, its answer as a JS value; an export's own
//  error (deUrb's, as the JS words it) is thrown as err(message)
export function call(fn, args, err = (m) => new Error(m)) {
  const b = enc.encode(JSON.stringify(args))
  const p = X.render_alloc(b.length)
  new Uint8Array(X.memory.buffer).set(b, p)
  const code = X[fn](p, b.length)
  const out = dec.decode(new Uint8Array(X.memory.buffer, X.render_out_ptr(), X.render_out_len()))
  if (code !== 0) throw err(out)
  return JSON.parse(out)
}
