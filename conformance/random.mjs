// Random page bodies and link targets, from a fixed seed so the file
// is the same each time, of the characters each renderer's rules turn
// on: Markdown's and gemtext's marks, JavaScript's whitespace and line
// terminators, astral characters, letters Go's (?i) folds (ſ, the
// Kelvin sign), ship names real and not, and the control characters
// safeLink refuses. gen.mjs writes them to inputs/random.json and the
// extension's readings to render-ext-random.json; the Go writes its
// own to render-go-random.json (conformance_test.go).

const CHARS = [...'##**`[]()_-+>1.', ' ', ' ', ' ', '\t', '\r', '\n', '\n', '\u00a0', '\u0085', '\ufeff', '\u2028', '\u3000',
  '\v', 'x', 'y', '/', ':', '\\', 'é', '\u{1F600}', '<', '=', 'h', 't', 'p', 's', '\x01', 'a', '\u017f', '\u212a', '&', "'", '"',
  '](', '**', '=>', '```', '* ', '- ', '1. ', '# ', '> ', 'urb://~', 'zod', 'nec', 'abc', 'marzod', 'dozzod', 'https://', 'HTTP',
  '%2e', '..', '?', '#', 'a'.repeat(100), '\u{1F600}'.repeat(60)];
const LINKS = ['urb://~zod', 'urb://~nec/x', 'URB://~zod/y', 'urb://~abc/x', 'urb://~dozzod/y', 'urb://~marzod', 'urb://~sampel-palnet/p',
  'HTTPS://a.b/c?d#e', 'http:/x', 'http://', 'httpſ://e.x', 'HTTPſ://e.x/', 'mailto:a@b', 'Gemini://g', 'javascript:x', '/', '//', '\\',
  '..', '.', 'x', 'y/z', '?q', '#f', '%41', '%2e', '%2E', 'é', '\u{1F600}', ':', 'a:b', '~', "'", '(', '!', '*', '"', '<', '{', '|',
  '^', '`', '[', ']', '@', '$', '&', '+', ',', ';', '=', '\x01', ' ', '\u00a0', '\u3000', '\u212a', '\x7f', 'a/../../b', './'];

export function randomRender() {
  let s = 0x2545f491;
  const next = () => { s ^= s << 13; s ^= s >>> 17; s ^= s << 5; return (s >>> 0); };
  const pick = (xs, n) => Array.from({ length: n }, () => xs[next() % xs.length]).join('');
  const bodies = [];
  for (let i = 0; i < 360; i++) bodies.push(pick(CHARS, next() % 60));
  for (let i = 0; i < 40; i++) bodies.push(pick(CHARS, next() % 600));
  const links = [];
  for (let i = 0; i < 600; i++) links.push(pick(LINKS, 1 + next() % 4));
  return { bodies, links };
}
