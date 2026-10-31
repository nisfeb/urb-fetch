// The extension's reading of each case, for TestExtensionParity:
//
//   node testdata/ext_parity.mjs <nisfeb-extension> <cases.json>
//
// prints [{md, gmi, html, safe, link}] as JSON: lib/markdown.js's blocks,
// lib/gemtext.js's blocks, looksHtml, safeLink, and where reader.js sends
// the case as a Markdown link on urb://~zod/a/b (the urb:// address, not
// the reader page's path).

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [ext, file] = process.argv.slice(2);
const lib = name => import(pathToFileURL(resolve(ext, 'lib', name)).href);
const { markdown, safeLink } = await lib('markdown.js');
const { gemtext, linkTarget } = await lib('gemtext.js');
const { looksHtml } = await import(new URL('../wasm/mesa.mjs', import.meta.url).href);

// reader.js's mdHref, which needs a page to run in
const page = 'urb://~zod/a/b';
const mdHref = target => {
  if (!safeLink(target)) return '';
  const t = target.replace(/^[a-z][a-z0-9+.-]*:/i, s => s.toLowerCase());
  return /^(mailto|gemini):/.test(t) ? t : linkTarget(page, t) || '';
};

const cases = JSON.parse(readFileSync(file, 'utf8'));
process.stdout.write(JSON.stringify(cases.map(c => ({
  md: markdown(c), gmi: gemtext(c), html: looksHtml(c), safe: safeLink(c), link: mdHref(c),
}))));
