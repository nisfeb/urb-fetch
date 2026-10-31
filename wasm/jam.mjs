// Nouns in plain JS, for the conformance vectors: an atom is a BigInt, a
// cell [head, tail]. jam and cue are Hoon's +jam and +cue (check.mjs
// checks jam against vere's), and asNoun gives a tree the interface
// mesa.mjs's decoders read (isCell, head, tail, size, bytes, text, int),
// with the same checks and messages as its Noun.

import { MAX_NUMBER, UrbError } from './mesa.mjs';

const hostile = what => new UrbError(`another ship's noun ${what}`);
const isCell = n => Array.isArray(n);
const bits = v => (v === 0n ? 0 : v < 0x20000000000000n ? Number(v).toString(2).length : v.toString(2).length);   // (met 0 v)

// the atoms and cells of a tree, each distinct one numbered once, so
// equal nouns are one noun, as +jam's map of nouns has it
function numberer() {
  const ids = new Map(), byNoun = new Map();
  const id = n => {
    if (byNoun.has(n)) return byNoun.get(n);
    const key = isCell(n) ? `${id(n[0])},${id(n[1])}` : `a${n.toString(16)}`;
    if (!ids.has(key)) ids.set(key, ids.size);
    const k = ids.get(key);
    if (isCell(n)) byNoun.set(n, k);   // a cell object is the same noun every time
    return k;
  };
  return id;
}

export function jam(noun) {
  const out = [];   // bits, low first
  const put = (v, n) => { for (let i = 0; i < n; i++) out.push(Number((v >> BigInt(i)) & 1n)); };
  const mat = a => {
    if (a === 0n) return put(1n, 1);
    const b = BigInt(bits(a)), c = bits(b);
    put(0n, c);
    put(1n, 1);
    put(b & ((1n << BigInt(c - 1)) - 1n), c - 1);
    put(a, Number(b));
  };
  const id = numberer(), seen = new Map();   // noun id -> bit offset
  const todo = [noun];
  while (todo.length) {
    const a = todo.pop(), k = id(a);
    if (seen.has(k)) {
      const at = BigInt(seen.get(k));
      if (!isCell(a) && bits(a) <= bits(at)) {
        put(0n, 1);
        mat(a);
      } else {
        put(1n, 1);
        put(1n, 1);
        mat(at);
      }
      continue;
    }
    seen.set(k, out.length);
    if (!isCell(a)) {
      put(0n, 1);
      mat(a);
      continue;
    }
    put(1n, 1);
    put(0n, 1);
    todo.push(a[1], a[0]);
  }
  const bytes = new Uint8Array(Math.ceil(out.length / 8));
  out.forEach((b, i) => { if (b) bytes[i >> 3] |= 1 << (i & 7); });
  //  an atom: no trailing zero bytes
  let end = bytes.length;
  while (end && !bytes[end - 1]) end--;
  return bytes.slice(0, end);
}

export function cue(bytes) {
  const len = bytes.length * 8;
  const bit = i => {
    if (i >= len) throw new Error('cue: past the end');
    return (bytes[i >> 3] >> (i & 7)) & 1;
  };
  const read = (at, n) => {
    let v = 0n;
    for (let i = n - 1; i >= 0; i--) v = (v << 1n) | BigInt(bit(at + i));
    return v;
  };
  const rub = at => {   // [bits used, value]
    let c = 0;
    while (!bit(at + c)) if (++c > 64) throw new Error('cue: a length too long');
    if (c === 0) return [1, 0n];
    const b = read(at + c + 1, c - 1) | (1n << BigInt(c - 1));
    return [c + c + Number(b), read(at + c + c, Number(b))];
  };
  //  depth-first with an explicit stack, so deep nouns don't overflow;
  //  refs holds each atom and cell by where it began, as +cue's map
  const refs = new Map(), cells = [];
  for (let pos = 0; ;) {
    const start = pos;
    let v;
    if (!bit(pos)) {
      const [n, a] = rub(pos + 1);
      pos += 1 + n;
      v = a;
      refs.set(start, v);
    } else if (!bit(pos + 1)) {
      pos += 2;
      cells.push({ start, head: null, full: false });
      continue;
    } else {
      const [n, a] = rub(pos + 2);
      pos += 2 + n;
      if (!refs.has(Number(a))) throw new Error('cue: a backref to nothing');
      v = refs.get(Number(a));
    }
    for (;;) {
      if (!cells.length) return v;
      const c = cells.at(-1);
      if (!c.full) {
        c.head = v;
        c.full = true;
        break;
      }
      cells.pop();
      v = [c.head, v];
      refs.set(c.start, v);
    }
  }
}

// bytes, little-endian, of an atom
const atomBytes = v => {
  const b = [];
  for (let x = v; x > 0n; x >>= 8n) b.push(Number(x & 0xffn));
  return Uint8Array.from(b);
};

// a tree read as mesa.mjs's decoders read a noun; a cell's reader is
// made once, so a shared tree is walked as fast as it can be
const readers = new WeakMap();
const dec = new TextDecoder();
class TreeNoun {
  constructor(t) { this.tree = t; }
  get isCell() { return isCell(this.tree); }
  get head() { if (!isCell(this.tree)) throw hostile('has an atom where a cell goes'); return asNoun(this.tree[0]); }
  get tail() { if (!isCell(this.tree)) throw hostile('has an atom where a cell goes'); return asNoun(this.tree[1]); }
  get size() { if (isCell(this.tree)) throw hostile('has a cell where an atom goes'); return Math.ceil(bits(this.tree) / 8); }
  bytes() { if (isCell(this.tree)) throw hostile('has a cell where an atom goes'); return atomBytes(this.tree); }
  text() { return dec.decode(this.bytes()); }
  int(max = MAX_NUMBER) {
    if (this.size > max) throw hostile(`has a number over ${max} bytes`);
    return this.tree;
  }
}
export function asNoun(t) {
  if (!isCell(t)) return new TreeNoun(t);
  let r = readers.get(t);
  if (!r) readers.set(t, (r = new TreeNoun(t)));
  return r;
}

export const hex = b => Buffer.from(b).toString('hex');
export const unhex = h => new Uint8Array(Buffer.from(h, 'hex'));
