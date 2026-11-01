//  gemtext.js on the Rust core: urb-render's ext (core/urb-render), in
//  render.wasm, with the extension's names and structures
//  (docs/rust-core.md, M6). The rules are the extension's lib/gemtext.js
//  of nisfeb-extension 78c30bb, which the conformance vectors hold the
//  Rust to.
//
//  gemtext(body): the page's blocks, {kind: 'h1' | 'h2' | 'h3' | 'p' |
//  'quote' | 'pre', text}, {kind: 'list', items} or {kind: 'link', href,
//  label}. linkTarget(base, href): where a link goes from the page at
//  base, an urb:// address or an http(s) url, or '' for nowhere.
//  linkHref(base, target): the reader's href for a link, or null.
//  readerPath(addr): the reader page for an urb:// address.

import { call } from './render.js'

export const gemtext = (body) => call('render_gemtext', [String(body)])
export const linkTarget = (base, href) => call('render_link_target', [String(base), String(href)])
export const linkHref = (base, target) => call('render_link_href', [String(base), String(target)])
export const readerPath = (addr) => call('render_reader_path', [String(addr)])
