//  markdown.js on the Rust core: urb-render's ext (core/urb-render), in
//  render.wasm, with the extension's names and structures
//  (docs/rust-core.md, M6). The rules are the extension's
//  lib/markdown.js of nisfeb-extension 78c30bb, which the conformance
//  vectors hold the Rust to: every link's target is checked by safeLink,
//  text is only ever text, and a body over MD_MAX is shown as written.
//
//  spans(line): [{text} | {code} | {strong} | {em} | {link, text}].
//  markdown(body): blocks, {kind: 'h1'..'h6' | 'p' | 'quote', spans},
//  {kind: 'ul' | 'ol', items}, {kind: 'hr'} or {kind: 'pre', text}.
//  mdNodes(doc, body, hrefFor): the page's DOM nodes, made with doc;
//  hrefFor(target) says where a safe link goes, or nothing to leave it
//  as text.

import { call } from './render.js'

export const MD_MAX = 256 * 1024

export const safeLink = (t) => call('render_safe_link', [String(t)])
export const spans = (line) => call('render_spans', [String(line)])
export const markdown = (body) => call('render_markdown', [String(body)])

//  the elements whose text the JS set as textContent, even when empty
const TEXT_SET = new Set(['pre', 'code', 'strong', 'em', 'a'])

//  The Rust gives the nodes, and this makes them with doc. hrefFor is
//  asked of each link once, in the JS's order, before any node is made:
//  render.wasm first lists the targets it will ask of, then takes the
//  answers. An answer never changes what is asked next.
export function mdNodes(doc, body, hrefFor) {
  const text = String(body)
  const hrefs = call('render_md_targets', [text]).map((t) => {
    const h = hrefFor(t)
    return h ? String(h) : null
  })
  const make = (n) => {
    if (typeof n === 'string') return n
    const e = doc.createElement(n.tag)
    if (TEXT_SET.has(n.tag)) e.textContent = n.kids.join('')
    else for (const k of n.kids) e.append(make(k))
    if (n.href !== undefined) e.href = n.href
    if (n.rel) e.rel = 'noopener noreferrer'
    return e
  }
  return call('render_md_nodes', [text, hrefs]).map(make)
}
