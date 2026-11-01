//  render.js: render.wasm (core/render-wasm), urb-render's extension
//  half and mesa.mjs's pure helpers, for the shims whose calls are
//  synchronous: gemtext.js, markdown.js, and mesa.mjs's renderAs,
//  looksHtml, deUrb, uv and da (docs/rust-core.md, M6).
//
//  The extension's service worker imports gemtext.js and mesa.mjs, and a
//  module service worker may not use a top-level await, so the module is
//  compiled synchronously, from the bytes render-wasm.js carries (built
//  by shim/build.sh). It is about 240 KB, under the 8 MB a browser lets a
//  page compile synchronously on its main thread. It is compiled on the
//  first call, not at import, so a browser without wasm fails only that
//  call (it throws, as each call after it does), not every module that
//  imports a renderer.

import RENDER_WASM from './render-wasm.js'
import { wasi } from './wasi.js'

let X = null, failed = null

function exports() {
  if (X) return X
  if (failed) throw failed
  try {
    const s = atob(RENDER_WASM), bytes = new Uint8Array(s.length)
    for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i)
    let x = null
    x = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
      wasi_snapshot_preview1: wasi(() => x.memory, console.error, {}, 'render.wasm'),
    }).exports
    X = x
  } catch (e) {
    failed = new Error(`render.wasm failed to start: ${e?.message ?? e}`)
    throw failed
  }
  return X
}

const enc = new TextEncoder(), dec = new TextDecoder()

//  fn(...args) in render.wasm, its answer as a JS value; an export's own
//  error (deUrb's, as the JS words it) is thrown as err(message)
export function call(fn, args, err = (m) => new Error(m)) {
  const X = exports()
  const b = enc.encode(JSON.stringify(args))
  const p = X.render_alloc(b.length)
  new Uint8Array(X.memory.buffer).set(b, p)
  const code = X[fn](p, b.length)
  const out = dec.decode(new Uint8Array(X.memory.buffer, X.render_out_ptr(), X.render_out_len()))
  if (code !== 0) throw err(out)
  return JSON.parse(out)
}
