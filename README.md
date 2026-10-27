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

Lattice labels every published page `%gmi`, whatever its format ([nisfeb/lattice#321](https://github.com/nisfeb/lattice/issues/321)). The gateway serves a page whose first character is `<` as HTML, and renders anything else as gemtext.

### Notes for other clients

Three things any client of grubbery's peek has to get right:

1. **Large files come back on a fixed wire.** A file over 64 refs or 8,192 bytes doesn't fit in the `%snap`, so the client sends `%want` and gets `%data`. Grubbery answers on the wire `/want`, not on the client's own wire, so the data has to be matched to its request by content hash.
2. **Keep one UDP port per identity.** Publishers send answers to the address they last saw. With a new port each run, they keep resending to a dead address, and newer answers wait behind those. The gateway keeps its port in the state folder.
3. **Never reuse a request wire.** Answers meant for an earlier run do arrive late, and a reused wire would hand a new request the wrong file. The gateway puts a random per-run prefix in every wire.

## Security

- It listens on loopback only. It refuses requests for any host name but its own, which blocks DNS rebinding.
- Each ship is its own origin, `<ship>.urb.localhost`, so one ship's pages can't script another's.
- A publisher's HTML is served with lattice's own sandbox policy. The page gets an opaque origin, so its scripts run but can't reach anything else.
- Gemtext is rendered by the gateway, with no scripts allowed.
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

Don't point `URB_FETCH_STATE` at a folder a running gateway is using.

## Prototype

`prototype/urb_fetch.py` is the first version, in Python on libmesa's ctypes binding. It reads pages from the command line, one process per run. It needs `LIBMESA=third_party/libmesa/zig-out/lib/libmesa.so` and `LIBMESA_PY=third_party/libmesa/bindings/python`.

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
