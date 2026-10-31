# urb-fetch

A local gateway to pages published with [lattice](https://github.com/nisfeb/lattice) on the Urbit network. It needs no Urbit ship. It runs as a comet, or as your moon, through [libmesa](https://github.com/mopfel-winrux/libmesa), and serves `urb://~sampel-palnet/page` at `http://sampel-palnet.urb.localhost:8475/page` for any browser.

Status: early. It reads published lattice pages and nothing else.

## Build

You need Go 1.26 or later and zig 0.15.2. libmesa pins that zig version and newer ones don't build it, so get 0.15.2 from [ziglang.org/download](https://ziglang.org/download/) if your package manager has moved on.

```
git clone --recursive https://github.com/nisfeb/urb-fetch
cd urb-fetch
make ZIG=/path/to/zig-x86_64-linux-0.15.2/zig
```

## Run

```
./urb-fetch
```

The first start mines a comet, which takes a few seconds, and keeps it in `~/.local/state/urb-fetch`. Then open `http://urb.localhost:8475/` and enter an `urb://` address, or go straight to `http://<ship>.urb.localhost:8475/<path>`. Add `?raw` to a page's address to see its source.

The first page from a ship takes a few seconds while the two ships meet. After that a page takes well under a second. Right after a restart, a ship can take a minute or two to answer, because it is still resending answers meant for the gateway's previous run. The gateway then says so (a 504), keeps the answer when it comes, and the next try is fast.

| Flag | Default | |
| --- | --- | --- |
| `-listen` | `localhost:8475` | Where to serve. `localhost` means both `127.0.0.1` and `::1`, since browsers try either for `*.localhost`. Keep it on loopback. |
| `-state` | `~/.local/state/urb-fetch` | The identity and its network state. |
| `-moon` | | Run as this moon instead of a comet. |
| `-new-moon` | | The moon has never run anywhere. Start its state empty. |
| `-relay` | | Run the relay for libmesa in a browser on this address, such as `localhost:8476`, instead of the gateway. See [wasm](#wasm). |
| `-relay-origins` | `chrome-extension://*` | The `Origin` patterns the relay accepts, comma-separated. Clients that send no `Origin`, such as Node, are always accepted. |
| `-fake` | | For tests: run the gateway as this ship on a fake network (`urbit -F`) on this machine. |
| `-relay-fake` | | For tests: relay to a fake network (`urbit -F`) on this machine, galaxy g at `127.0.0.1:31337+g`. |

### As your moon

A comet is anonymous. To read as a moon of your own ship instead, run `|moon` in your ship's dojo, save the key it prints as a one-line file with mode 600, and start with:

```
./urb-fetch -moon moon.key -new-moon
```

Pass `-new-moon` only the first time, and only for a moon that has never booted anywhere else. An identity must never run from two copies of its state, so a moon that has run elsewhere needs `|moon-breach` on its parent first. Your ship has to be online, because other ships learn a moon's keys from its parent. A moon reads what publishers share with the moon itself. It doesn't get your ship's access.

## What it reads

- A ship's front page: `urb://~sampel-palnet`.
- Published pages: `urb://~sampel-palnet/notes/intro` and `urb://~sampel-palnet/n/notes/intro`. A folder address such as `urb://~sampel-palnet/site/` reads `site/index`.
- Only pages the publisher shares with everyone.
- Only publishers on lattice release 35 or later, which publish a revision list.

The `/p/`, `/k/` and `/t/` forms name the publisher's own tree, which a stranger can't read. The gateway answers them with 501.

## How it works

It reads a page the way lattice's own reader does (`nex/lattice/app.hoon`: `+read-page-or-index`, `+read-page-body-rev`):

1. Resolve the address as lattice's `+de-urb` does.
2. Read the publisher's revision list, the grubbery file `<base>/pub/revs`, with grubbery's cross-ship peek. That is a `%grubbery-load` poke of `[[wire dest] %peek ~ %.y]` to the publisher's `%grubbery`, answered by a `%grubbery-transfer` poke back. The list is kept for 60 seconds.
3. Read the page body by remote scry at `/g/x/1/grubbery//1/pub/page/<path>/<id>`, with the id from the list.
4. If the page isn't on the list, or the scry gets no answer within 10 seconds, peek the page's file directly.

The ids are salted on purpose. A publisher's runtime keeps answering any scry path it has answered once, even after the page changes, so guessable paths would let anyone read old revisions.

A page renders by the mark it was published under, as lattice's reader does. Lattice release 74 and later publish each page under its format's mark: `%md`, `%html`, `%text`, `%gmi`, `%js`, `%css` or `%noun`. Older releases publish every page as `%gmi`, whatever its format ([nisfeb/lattice#321](https://github.com/nisfeb/lattice/issues/321)), and the vault file read in step 4 is `%gmi` on every release. So a `%gmi` page whose first character is `<` is HTML, and any other `%gmi` page is gemtext, as before.

| Mark | Served as |
| --- | --- |
| `%html` | the publisher's HTML, in the sandbox |
| `%md` | Markdown, rendered by the gateway |
| `%gmi` | gemtext, or HTML in the sandbox if it starts with `<` |
| `%text`, `%js`, `%css`, `%noun` | text, in a `<pre>` |

The gateway renders a small, safe subset of Markdown itself, since it has no Markdown library: headings, paragraphs, lists, quotes, rules, fenced code, inline code, links, bold and italics. Everything is escaped, so raw HTML in Markdown shows as text. Gemtext, Markdown and text read by the same rules as [nisfeb-extension](https://github.com/nisfeb/nisfeb-extension)'s reader (`render.go`): links only to the web, `urb://`, gemini, mail, or a path on the page's ship, resolved against the page; a Markdown body over 256 K UTF-16 units is shown as text. The `X-Urb-Mark` header names the mark.

Release 74 also rotated every publisher's salt once, so every page's id changed. The ids come from the revision list, so a reader only needs a fresh list, which it reads every minute.

### Notes for other clients

Three things any client of grubbery's peek has to get right:

1. **Large files come back on a fixed wire.** A file over 64 refs or 8,192 bytes doesn't fit in the `%snap`, so the client sends `%want` and gets `%data`. Grubbery answers on the wire `/want`, not on the client's own wire, so the data has to be matched to its request by content hash.
2. **Keep one UDP port per identity.** Publishers send answers to the address they last saw. With a new port each run, they keep resending to a dead address, and newer answers wait behind those. The gateway keeps its port in the state folder.
3. **Never reuse a request wire.** Answers meant for an earlier run do arrive late, and a reused wire would hand a new request the wrong file. The gateway puts a random per-run prefix in every wire.

## Security

- It listens on loopback only. It refuses requests for any host name but its own, which blocks DNS rebinding.
- Each ship is its own origin, `<ship>.urb.localhost`, so one ship's pages can't script another's.
- A publisher's HTML is served with lattice's own sandbox policy. The page gets an opaque origin, so its scripts run but can't reach anything else.
- Gemtext, Markdown and text are rendered by the gateway, with no scripts allowed. HTML is never served outside the sandbox, whether its mark says so or an older `%gmi` page starts with `<`.
- It only reads. It accepts GET and HEAD and nothing else.
- `comet.key` in the state folder is a secret.
- Every publisher you read keeps a record of your comet. Keep the state folder rather than starting fresh, or each fresh start leaves a new comet behind on every ship you visit.

## Tests

```
make test
```

`TestLive` checks the protocol against a real publisher. It is skipped unless you name one. It reads every page on the list both ways, by scry and by peek, and expects the same text:

```
URB_FETCH_LIVE=~sampel-palnet URB_FETCH_STATE=/some/dir go test -run Live -v
```

Don't point `URB_FETCH_STATE` at a folder a running gateway is using. `URB_FETCH_FAKE=~ship` reads as that ship on a fake network instead.

## Prototype

`prototype/urb_fetch.py` is the first version, in Python on libmesa's ctypes binding. It reads pages from the command line, one process per run. It needs `LIBMESA=third_party/libmesa/zig-out/lib/libmesa.so` and `LIBMESA_PY=third_party/libmesa/bindings/python`.

## wasm

`wasm/` builds libmesa as WebAssembly, so a browser or an extension can read `urb://` pages itself.

- `build.sh` applies `libmesa-wasm32.diff` to a copy of libmesa and links it with `glue.c` into `build/mesa.wasm`.
- `mesa.mjs` runs the engine as a comet and reads pages the way the gateway does. `read` gives each page's `mark` with its body, and `renderAs(mark, body)` says how to render it: `html` (sandboxed), `md`, `gmi` or `text`, as the gateway does. It works in Node and in a browser. Its host supplies a transport for datagrams and a store for the comet and the engine's state. Each commit's changes are saved before the datagrams it released go out.
- `host.mjs` reads one page from Node, and `node.mjs` has what Node's command lines share: the state folder, UDP, and the options that pick who runs.
- `furum.mjs` talks to [furum](https://github.com/nisfeb/furum) boards on another ship: it reads a board's posts, comments and votes, and posts, comments and votes through the host's public `inbox.sig`. `furum-cli.mjs` drives it from Node.
- `pub.mjs` publishes pages from the engine, each under its format's mark as lattice 74 and later do (`pub-cli.mjs` takes it from the file's extension: `.md`, `.html`, `.txt`, `.js`, `.css`, else gemtext), so lattice readers on other ships open them at `urb://~our-ship/<name>`. It lays them out as lattice does: `/pub/index` and `/pub/revs` and each page's vault file answer grubbery peeks, a keep on a page's vault file hears its edits, and each revision's body answers remote scry at an unguessable id. `pub-cli.mjs` publishes and serves from Node.
- `check.mjs` checks serving with no ship. vere's `urbit eval` computes the hashes, maps, ids and dates that other ships check, and the engine must agree. Then two engines in one process publish and read pages, edits and removals.
- `test/` holds the unit tests, run with `node --test wasm/test/` after `build.sh`. `open` takes optional `now` and `entropy` hooks, the clock and the random bytes, for tests and recordings: with both fixed, a session sends the same datagrams every time.
- Every noun from another ship is read under caps, written at the top of `mesa.mjs`'s hostile nouns section: 200,000 cells counted as a tree, as lattice's `+small-noun` does, `$json` 64 deep, numbers 32 bytes, paths 64 segments. A noun past one, or of the wrong shape, fails the read that waits on it with an `UrbError`; `test/hostile.test.mjs` sends them by hand.

```
ZIG=/path/to/zig-x86_64-linux-0.15.2/zig wasm/build.sh
node wasm/host.mjs urb://~sampel-palnet/page
```

The comet is mined on the first run and kept in `~/.local/state/urb-fetch-wasm`, with its state and its UDP port. `--tries 4` keeps the engine running across several tries, for a ship that is still catching up. `--trace` logs every datagram and the engine's view of it.

`--moon moon.key` reads as a moon instead, from the key `|moon` printed, with its state in a folder of its own. Pass `--new-moon` on its first run only, and only for a moon that never ran anywhere, as with the gateway's `-new-moon`. A moon, or a comet, can also read what a publisher shares with it privately: `urb://~ship/k/<key>` reads a know entry, and `urb://~ship/t/<path>` a text file anywhere in the publisher's tree. The publisher decides who may read each, so anything not shared looks the same as nothing there. `urb://~ship/p/...` names a live Lattice page, which only Lattice shows.

Apps on other ships: `urb.grub.peek` reads a file in a ship's grubbery, `urb.grub.poke` pokes one of its grubs, and `urb.grub.onInbox` hears grubs that poke ours. A poke resolves once the grub took it in, or with the refusal. `furum.mjs` is built on these.

Serving: `urb.grub.serve` answers other ships' grubbery peeks and keeps of the files the host names, and `urb.scry.serve` answers remote scry. A reader's ship checks that every noun in a grubbery answer hashes to its name, and clams every map against its exact treap shape, so the engine builds both as Hoon does. `pub.mjs` is built on these:

```
node wasm/pub-cli.mjs --put notes/intro=intro.gmi
URBIT=/path/to/urbit node wasm/check.mjs
```

The pages, and the keeps of readers following them, are kept in `pages.json` in the identity's state folder. Serving lasts while the engine runs. A reader that read a page in the last minute keeps seeing the revision it read, as with lattice: its list of ids is a minute old. Tested on 2026-10-28 on fake ships, with lattice main reading a fake moon: pages, the home page and a 15 KB page by remote scry, the 15 KB vault file inline over peek, and a missing page as a 404. A page followed from lattice heard an edit, a removal and a republish, and the reader's drop.

`--fake ~ship` runs on a fake network on this machine (`urbit -F`), with keys made from the ship's name, for testing against fake ships. On furum's fleet set (`~rup` hosting board `urbtest`):

```
node wasm/furum-cli.mjs --fake ~dozbud-dozzod-dozhus ~rup urbtest --post 'Hello' --body 'From a moon in wasm'
```

### The relay

A browser can't send UDP. The relay sends and receives a browser's datagrams for it over a WebSocket:

```
./urb-fetch -relay localhost:8476
node wasm/host.mjs --relay ws://localhost:8476 urb://~sampel-palnet/page
```

The relay keeps one UDP port for each ship, open while it runs and the same across runs. It looks up galaxies' addresses for the browser. It sees which ships a browser talks to and the public pages it reads, but never a key. It accepts any ship name and sends datagrams anywhere, so keep it on loopback. Hosting it for others needs proof that a client holds its ship's key, and rate limits.

When a comet moves to a new address, its publishers keep resending earlier answers to the old one for a minute or two before newer answers arrive. Keep a comet on one transport.

Tested on 2026-10-27 against a live ship, over UDP and through the relay: the same comet across runs, pages read by remote scry with the same text the gateway serves. The paths for large files and the fallback to the publisher's vault are ported from the prototype but have not run yet.

## Ladybird

The `urb` branch of [nisfeb/ladybird](https://github.com/nisfeb/ladybird/tree/urb) is the Ladybird browser with `urb://` support. It opens `urb://` addresses through this gateway, from links, scripts, forms and its address bar, and its address bar shows a gateway page by its `urb://` address. Set `LADYBIRD_URB_FETCH_PORT` if the gateway runs on another port.

```
git clone -b urb https://github.com/nisfeb/ladybird
cd ladybird
./Meta/ladybird.py build
```

Tested on Linux against live ships on 2026-10-27, with Ladybird's sandbox on:

- `TestURL`, with a new case for both directions of the mapping.
- An `urb://` address given on the command line rendered through the gateway.
- A page whose script navigates to an `urb://` address reached the gateway, both on the default port and with `LADYBIRD_URB_FETCH_PORT`.
- Not yet tried in the windowed browser, so the address bar's display of `urb://` addresses is covered by the unit test only.

## License

MIT. libmesa, a submodule, is MIT too.
