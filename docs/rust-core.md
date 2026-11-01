# The Rust core: plan

One implementation of urb-fetch's protocol logic and of urb:// rendering, in Rust, on top of libmesa's C engine. It runs native in the desktop app, Ladybird and a command-line tool, and as wasm in the extension, the web UI and a PWA. It replaces the logic in `wasm/mesa.mjs`, `wasm/pub.mjs` and `wasm/furum.mjs`, the Go gateway's reader (`reader.go`, `urb.go`), and the renderers in the Go gateway and the extension (`lib/gemtext.js`, `lib/markdown.js`).

The work happens on the `rust-core` branch. Nothing reaches users until every gate below passes. The rule above all others: **no regressions.** Every behaviour users have today must survive the switch unchanged, including state they already hold.

## Decisions of 2026-10-30

These follow the review of the first draft (lattice wiki page `reports/2026-10-30-rust-core-plan-review`). Each one is decided 2026-10-30.

1. **Pins.** Lattice release 81. Grubbery b126c50 for `~ricsul-bilwyt` and eed73a6 for the fleet. Furum release 7. See "The pins" below for how each was found.
2. **Lattice forums are in scope, as a reader.** Trace 8 and its vectors cover them.
3. **Rendering is in scope.** A third crate, `urb-render`, holds gemtext, lattice's Markdown subset, text, and the gateway's HTML sandbox and CSP rewriting. Milestone M5b builds it. Lattice's own Hoon renderer is a non-goal.
4. **Trace 9** covers pages published by lattice 74 and later, and a page regrown under a rotated salt.
5. **Re-recording is a CI job.** After an upstream change, CI fails if the traces drift without a commit that updates them.
6. **The desktop.** Lattice desktop (Tauri) is a later consumer. It links the native host as a Rust crate. It is outside the gates.
7. **Effort.** 5 to 6 weeks plus the soak, with M1 at about a week.
8. **Where it lives.** urb-fetch (open decision 1, now closed).

## The pins

- **Lattice release 81** (`7584d6a` on lattice's main, "quick Back and a steady bookmark star"). Since release 74 each page grows under its format's mark (md, html, text, css, js or gmi), and the salt was rotated, so revision ids changed. urb-fetch main (PR #1, `162206d`) and the extension already read every view-mode mark. Since release 73 lattice serves forum boards over urb://.
- **Grubbery b126c50 on `~ricsul-bilwyt`.** The ship has no version or commit marker: its docket says version 1.1.0, which is lattice's docket, not a commit. So the commit was found by fingerprint, read-only, from the ball's mirror of the clay desk (`/grubbery/api/file/sys/clay/desks/grubbery/...`). Of the 107 desk files outside `gub/`, 104 match b126c50 exactly, including `app/grubbery.hoon`, `lib/nexus.hoon` and `lib/tarball.hoon`. The other three are the docket and two test files. Of the files that changed between eed73a6 and b126c50, `gub/mar/clay/trunk/trunk-action.hoon` and `gub/nex/shell.hoon` match b126c50 and not eed73a6. No later commit matches.
  - **The whole desk, 2026-10-31 (M1):** all 883 files, `gub/` included, read with GETs only, scored against all 1,174 commits in grubbery's repository (origin and fork). The mirror serves a `.md` without its last newline and a `.json` re-serialised, so those compare as text without trailing newlines and as parsed JSON. b126c50 holds 699 desk files. 697 are on the ship, and 696 match; the 697th is the docket, which is lattice's. The two it holds that the ship lacks are `gub/nex/git/forge/CLAY.md` and `gub/nex/sys/WEBSOCKET.md`: clay can't hold an upper-case name. The ship has 186 files b126c50 lacks (184 in `gub/`, such as `gub/lib/mcp/`, and the two catalog-analyzer tests). Each matches a file an older commit had, so they are files grubbery removed that the ship's desk kept. Of the 1,174 commits b126c50 is the only one with no file that differs from the ship. **The pin stands.** Fingerprint it again whenever the ship takes an update.
- **Grubbery eed73a6 on the fleet,** from `hoon-development/platform/grubbery.ref` (`nisfeb/grubbery eed73a6bfb8c`). `lib/nexus.hoon`, `lib/tarball.hoon` and `app/grubbery.hoon` are the same at both commits, so the clam molds are the same for both.
- **Furum release 7** (`94cfa61`, `code/version.json` says 7), still the latest on furum's master.

## What exists today

| Piece | Language | What it does | Tested by |
|---|---|---|---|
| libmesa 2cbb076 | C | the mesa protocol engine, sans-IO | its own suites |
| `wasm/glue.c` | C | libmesa as a wasm module: hooks, the row-journaling store, sham, mug | `check.mjs` |
| `wasm/mesa.mjs` | JS | the engine's host, nouns, the lattice reader, the grubbery client and server | `check.mjs`, the extension's harnesses, the fake fleet |
| `wasm/pub.mjs` | JS | lattice's publishing layout | `check.mjs`, fleet runs |
| `wasm/furum.mjs` | JS | furum boards: read, post, comment, vote, notes | the extension's furum harness |
| `node.mjs`, `host.mjs`, `pub-cli.mjs`, `furum-cli.mjs` | JS | Node hosts and command lines | by hand, fleet runs |
| `main.go`, `urb.go`, `reader.go` | Go | the HTTP gateway, its own lattice reader, and its renderers: gemtext, Markdown, text, HTML rewriting, the sandbox CSP | `urb_test.go`, `reader_test.go`, `live_test.go` |
| the extension's `lib/gemtext.js`, `lib/markdown.js` | JS | the extension's renderers, as parsed structures the reader builds into DOM nodes | the extension's `npm test` |
| `relay.go` | Go | the WebSocket relay for browsers | by hand, the harnesses |

The relay stays in Go. It is not part of this work.

Nothing in urb-fetch reads a lattice forum today. The JS reader for it is written on main first (see M1), so that the Rust has a reference to port.

## Goals

- One implementation of the protocol logic for every consumer.
- One implementation of urb:// rendering for every consumer that renders: the gateway and the extension.
- The same observable behaviour as today, byte for byte where a byte is observable: what goes on the wire, what is saved, what each API returns, what each error says, and what each renderer produces.
- Existing identities keep running. A comet or moon that ran under the JS engine or the Go gateway continues under Rust from its saved state, with no breach.

## Non-goals

- **No new features during the port.** Live following, gall pokes, private publishing, a hosted relay and the desktop's shipless mode all wait until the port has shipped. Each gets its own plan. The lattice forum reader is the one exception, and it is written in JS on main first.
- **No protocol changes.** The shapes stay as lattice release 81, grubbery b126c50 and eed73a6, and furum release 7 expect them.
- **No rewrite of libmesa.** It stays C, pinned at 2cbb076 with `libmesa-wasm32.diff`. A libmesa upgrade is a separate change, after the port.
- **Lattice's own Hoon renderer.** Lattice renders its pages in Hoon on its own ship. That stays as it is.
- **Writing to lattice forums from the Rust core** beyond what trace 8 records. The reader posts and hears refusal notes, as the JS does. Board management is not ported.
- **The desktop in the gates.** Lattice desktop is a later consumer. It is not a gate for any release here.
- **The relay stays as it is.**

## Rules for the port

1. **Port, don't improve.** The JS is the specification. If the Rust would behave differently, that is a bug in the Rust, even where the JS looks wrong. For rendering the Go gateway is the specification of the gateway's HTML, and the extension's JS of the extension's.
2. **A bug found in the JS or the Go is fixed there first,** on main, with a test, and then ported. The reference stays correct, and the fix reaches users without waiting for the port.
3. **Every observable is written down before it is ported:** wire shapes, saved formats, API signatures, error text, timeouts and their order. The list below is the start.
4. **Every input from another ship is hostile.** Decoders bound their depth and size, as lattice does with its 200,000-cell cap, and they never panic. Today's JS decoders (`entries`, `list`, `json`, `fileOf`) have no such caps; only the tang printer stops at depth 32. So, by rule 2, the caps go into the JS first, on main, and the Rust ports the hardened JS. A page body is hostile input to the renderers too.
5. **Small reviewed steps.** One milestone at a time, each reviewed before the next starts.

## The compatibility contract

These must not change. Each one gets a test in the conformance suite or a gate.

**Saved state**

- The extension's IndexedDB layout: database `urb` for the comet and `urb-<moon>` for a moon, stores `rows` and `meta`, meta keys `keyfile`, `fresh` and `pub`. Rows are glue.c's `[klen:4][key][vlen:4][val]`, little-endian, as libmesa's memory store writes them.
- `meta.pub` is `{pages: {'/<name>': {body, rev, id, at, hash}}, keepers: [{ship, wire, folder, name}]}`.
- Node's state folders: `comet.key`, `rows`, `port`, `pages.json`.
- The Go gateway's state: `comet.key`, `<ship>.db` and `<ship>.db.port`. The Go binding's SQLite store uses the same schema as libmesa's C store (one `mesa_kv` table of `k BLOB PRIMARY KEY, v BLOB`, WAL, `synchronous=FULL`, a lock on `<path>.lock`), checked in both sources, so the native host can open the file as it is. The migration gate still tests it.
- A moon may start from empty state only while `fresh` is set. Rust must refuse an empty store for a moon that ran, with the same message.
- One UDP port per identity, kept. A new port stalls every publisher for about 80 seconds.

**The module interface the extension imports**

- `mesa.mjs`: `open({wasm, transport, store, log, env, fake})`, `relayTransport`, `readRows`, `writeRows`, `looksHtml`, `keyfileInfo`, `fakeKeyfile`, `shipName`, `uv`, `da`, `DESK_BASE`, `deUrb`, `UrbError`.
- The engine object: `our`, `read(url)`, `grub.peek`, `grub.poke`, `grub.onInbox`, `grub.serve`, `grub.changed`, `scry.serve`, `nouns.sham`, `close()`.
- `furum(urb, host)`: `nextPost`, `posts`, `comments`, `votes`, `newPost`, `newComment`, `upvote`, `onNote`, with today's result shapes.
- `publish(urb, {pages, keepers, save})`: `pages`, `put`, `remove`; and `pageKey`.
- The lattice forum reader's interface, once it lands on main in M1.
- `gemtext.js`: `gemtext`, `linkTarget`, `readerPath`. `markdown.js`: `spans`, `markdown`, `mdNodes`, `safeLink`, `MD_MAX`. Same names, same returned structures.
- Errors: `UrbError` with `later` where it is set today, and the same message text. The extension's pages and tests match on some of it, for example `not published`.

The new wasm build ships with JS shims that keep these exact names and shapes, so `lib/urb.js` and `reader.js` in the extension do not change.

**On the wire**

- Every noun we send: grubbery `%grubbery-load` peeks, pokes and their wires (`/urb-fetch/<nonce>/<n>`, `/grub-poke/<nonce>/<n>`), `%pack` answers, `%want`, `%snap` answers with their silo, `%grubbery-intake` waves, and remote scry paths.
- Lattice forums: the deep peeks of a board's `/forum/boards/<board>/pub` and `/content`, the `[/lattice %forum-action]` poke to the host's `/forum.sig`, and the refusal note `[host board reason at]` the host pokes back.
- Which flow each goes on: one shared flow per ship for peeks and answers, a flow of its own per poke, corked once answered.
- Timing: keen 10 s, then `%yawn`; peek 30 s; the revision list cached 60 s; `close()` waits 500 ms after the last save.

**Rendering**

- Which renderer a page gets, by its mark: `renderAs` in `urb.go` and its match in the extension. Only a `%gmi` page is sniffed for HTML.
- Gemtext, Markdown and text, as the Go gateway writes them in HTML and as the extension's JS parses them.
- The hostile-input rules: everything escaped; links only to `http`, `https`, `urb`, `gemini`, `mailto` or a path on the page's ship, with schemes matched in any case; no protocol-relative `//` or `\\`; no target with a C0 control, a space or DEL in it; link text at most 256 and target at most 512; a Markdown body over 256 KB shown as text. These are the extension's rules as of nisfeb-extension 800ff22.
- The gateway's headers: the sandbox CSP for a publisher's HTML (`sandbox allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation`), and `default-src 'none'; style-src 'unsafe-inline'` for what the gateway renders itself. `rewriteHTML`'s urb:// link rewriting. The per-ship origin, `<ship>.urb.localhost`.

**The Go gateway's HTTP behaviour,** for when the Rust binary replaces it: hosts and routes, headers, rendering, and error pages.

## Architecture

Three crates in a Cargo workspace at `core/` in this repository:

- **`mesa-sys`** builds libmesa from `third_party/libmesa` with the `cc` crate, with the same explicit flags as `build.zig` (`-std=gnu23`, `-DU3_OS_ENDIAN_little=1`, `-DU3_OS_PROF=1`, the OS define, the urcrypt subset) and the wasm diff applied for wasm builds. The toolchain differs from zig's, as M0's findings set out: the clang on PATH, not zig's own clang 20, and baseline x86-64 where zig defaults to the build machine's CPU. Its bindings are generated by `bindgen` from `include/mesa/*.h`, never declared by hand: a signature that drifts from the header is silent undefined behaviour. It holds no logic.
- **`urb`** holds the logic and both hosts:
  - **The core** is sans-IO, like libmesa. It owns the engine, takes `now` and entropy from its host, and turns inputs (a datagram, a tick, a key lookup's answer, a request from the app) into outputs (datagrams, rows to save, events). It never opens a socket, a file or a timer. It reports one deadline: the earliest of libmesa's and its own timeouts.
  - **Hooks** are trampolines from C into Rust. Each one catches any panic (`catch_unwind`) and returns that hook's default, as `guarded()` does in the JS: a panic unwinding into libmesa's C frames is undefined behaviour.
  - **Nouns** are a safe wrapper: an owned noun frees itself, a borrowed one cannot outlive its hook. Maps and sets are built as `+put` builds them. `sham` and `mug` come from libmesa, which agrees with vere where a literal reading of `+shas` does not.
  - **The native host** (feature `native`) drives the core with a UDP socket, a timer, an HTTPS client for the roller and bootstrap, and libmesa's SQLite store. It is the library the desktop app and Ladybird link, and the command-line tool's engine.
  - **The wasm host** (feature `wasm`, target `wasm32-wasip1`) exports a small set of plain functions. The JS shims (`mesa.mjs`, `pub.mjs`, `furum.mjs`, now thin) give the transport, the store, fetch and timers, and keep today's module interface.
- **`urb-render`** is pure Rust, with no C and no IO. It takes a body, its mark and a link function, and gives either the parsed structure the extension builds nodes from or the HTML the gateway serves. It holds gemtext, lattice's Markdown subset, text, `renderAs` and HTML sniffing, `rewriteHTML`, and the CSP choice. The extension uses it through the wasm build. The gateway's replacement (M8) uses it natively. It does not depend on `urb`.

Request bookkeeping that the JS keeps in Maps today (peeks, pokes, keens, the revision cache, keepers) moves into the core as plain state with explicit timeouts. Requests are ids, and their completions are events. JS wraps them in promises. Native code wraps them in channels or futures.

## Making it testable before writing it

These come first, because they are what proves the port matches.

**Injectable clock and entropy in the JS.** `mesa.mjs` and `pub.mjs` use `Date.now()` and `crypto.getRandomValues` for wire nonces, snap ids, page ids and comet mining. Add optional `now` and `entropy` hooks to `open()`, defaulting to today's sources. This is a JS change on main, with no behaviour change, released on its own first.

**Hardened decoders in the JS.** Depth and size caps on every decoder of another ship's data (see rule 4), released on main on their own, with tests.

**The lattice forum reader in JS.** Read a public board and a thread over urb://, post through the host's `/forum.sig`, and hear refusal notes, as lattice 81 does. Released on main on its own, with tests against the fleet's lattice.

**One set of rendering rules.** The Go gateway's `safeLink` and Markdown regexes do not yet have 800ff22's rules: no control-character check, schemes matched in lower case only, unbounded link text and target, and no 256 KB cap. A target like `\x01//evil.com` passes Go's `safeLink` today. By rule 2 the Go gets the same rules on main first, with tests. Any other difference between the Go and the JS renderers is listed, and each is settled on main by making one match the other, before M5b ports them.

**Recorded sessions.** libmesa's engine is deterministic: it reads no clock and no OS randomness (confirmed in M0; see "M0's findings"). Time enters only as the `now` its calls take, and entropy only as bytes its caller passes in: the `eny` comet mining takes, and the key `mesa_shut_add` takes. One environment variable changes what it does: `MESA_TRACE` turns its tracing on, so the recorder and the replay run with it unset. So with the clock and entropy fixed, one session's inputs fully decide its outputs. A recorder wraps the JS host and writes every input and every output to a trace file:

- inputs: datagrams heard, key lookups and their answers, app calls, entropy drawn, and every timer firing (the engine's wake, and each keen, peek and poke timeout) as a tick with its `now`
- outputs: datagrams sent, rows saved, events, API results

"Byte for byte" means: replaying the trace into the Rust core, the outputs of each input step equal that step's recorded outputs. Timeouts compare their deadlines with `>=` on the same numbers as the JS, so a tick lands on the same side of every deadline.

Traces to record, each against the fake fleet:

1. a cold comet reading a lattice page, the home page, a 15 KB page, and a missing page
2. a moon's first run (`fresh`), then a second run from saved state
3. private reads: a know entry and a tree file, granted and not granted
4. furum: list, thread, post, comment, upvote, and a refusal note
5. publishing: index and revs peeks, a keen, a keep, an edit, a removal, a republish, and the reader's drop
6. timeouts: a keen that misses and is yawned, a peek nobody answers, a poke with no `%pack`
7. a page over 8 KB fetched through the `%want` leg (needs a publisher that answers that way: real grubbery on a fake ship)
8. a lattice forum over urb://: a public board read, a thread read, a cross-ship post that lands, and one the host refuses, with its refusal note
9. a reader fetching md, html and text pages published by lattice 74 or later, and a page regrown under a rotated salt

Traces are recorded against lattice 81 on the fleet. Traces 8 and 9 need a fleet set on lattice 81 (`hus-lattice` is updated to it in M1).

**Re-recording in CI.** A CI job records every trace again against the fleet at the pinned releases and compares them with the committed traces. A drift with no commit that updates the traces fails the job. After an upstream release the order is: update the pins and the JS on main, re-record, commit the new traces in the same change, then let the Rust follow. The job needs the fleet, so it runs on a self-hosted runner on this machine. A runner without the fleet skips it and says so, and never passes it.

**The conformance suite.** Language-neutral test vectors, generated from the JS and the Go: each is an input (a JS value, a url, a noun as jam hex, a hostile noun, a page body) and the expected output (jam hex, a parsed value, HTML, an error). The JS and the Go run them now. The Rust must pass every one. They cover:

- `deUrb` (from `prototype/test_de_urb.py`, `urb_test.go` and the JS), including the `/forum/<board>[/<post>]` form
- noun building: atoms, cords, cells, lists, paths, maps and sets of every key kind we use
- `sham`, `mug`, `uv`, `da`, checked against vere's `urbit eval` as `check.mjs` does now (1,029 cases, more added)
- every message builder above, and every decoder: snaps, silos, `%data`, `%pack`, `%veto`, `%miss`, waves, json, tangs, furum's records, lattice's forum board and thread grubs, its `%forum-action` and its refusal notes
- the renderers: `renderAs` for every mark, gemtext and Markdown from the extension's `gemtext.js` and `markdown.js` (the parsed structures) and from the Go gateway's `renderGemtext`, `renderMarkdown`, `renderText` and `rewriteHTML` (the HTML), with the cases in `urb_test.go` and the extension's `test/` as the start
- hostile inputs: deep nesting, huge declared sizes, shared structure, wrong tags. The expected result is an error, never a crash or a hang. These vectors are written by hand against the hardened JS, not generated, since they test the caps. For the renderers: every link the allowlist refuses (`javascript:`, `data:`, `vbscript:`, mixed case, `//`, `\\`, each C0 control, space and DEL at the start, middle and end), raw HTML, link text and targets just under and over their caps, a Markdown body just under and over 256 KB, and lines that make a backtracking regex slow.

**The composed shapes, checked by Hoon itself.** `check.mjs` checks primitives against vere today. Add grubbery's molds (`transfer`, `snap`, `silo`, `ject`, `leaf`, `lobes`, `wave`, and the `rail`, `blot` and `lane` they use) to its eval expressions, and clam every message we build against them, the way the receiving ship will. The molds are copied verbatim from `lib/nexus.hoon` and `lib/tarball.hoon` at the pinned grubbery commits. Those two files are the same at eed73a6 and b126c50, so one copy serves both. Lattice's `action:ft` for forums is added the same way, from lattice 81.

## Milestones

Each has exit criteria. A milestone isn't done until they all pass and the change has been reviewed.

**M0. Groundwork** (2 days)
- The workspace, `mesa-sys` building libmesa natively and for wasm32-wasip1.
- libmesa's own test suites run against our build, natively and under WASI (11 of 12 pass under WASI; store_test does not compile there, as wasi-libc has no mkdtemp).
- Confirm the engine reads no clock or randomness of its own.
- CI runs `cargo test` and `check.mjs`. The conformance suite joins CI in M1, when it is written.
- Exit: libmesa's suites pass with the same results as its zig build.

**M1. The test bed, in JS** (about a week)
- Fingerprint `~ricsul-bilwyt`'s whole grubbery desk, read-only, and confirm b126c50 or update the pin.
- Injectable clock and entropy in `mesa.mjs`, and the hardened decoders, released on main on their own.
- The lattice forum reader in JS, released on main on its own.
- The Go renderers given 800ff22's rules, and the other renderer differences settled, on main.
- `npm run urb` given a fleet target (`hus-lattice` serves lattice, updated to release 81), so routine runs stop mining a comet against `~ricsul-bilwyt` each time.
- The recorder, and traces 1 to 9.
- The conformance vectors, generated, passing in JS and Go, and run in CI.
- The grubbery molds and lattice's forum action added to `check.mjs`.
- The re-recording CI job.
- Exit: everything above runs in CI, and every trace replays into the JS itself byte for byte (the recorder's own check).

**M2. Nouns and codecs** (2 days)
- The noun wrapper, maps and sets, json, tangs, paths, sham, uv, da.
- Property tests: random maps and sets must match vere's `malt` and `silt` by hash; random JS-shaped values must round-trip.
- A live-noun counter in tests: every test ends with zero nouns alive.
- Exit: all codec vectors pass; the counter stays at zero; fuzzing the decoders finds nothing in a long run.

**M3. The engine and the reader** (3 days)
- The sans-IO core around libmesa, with its timeouts.
- The grubbery client: peeks on the shared flow, pokes on flows of their own, `%pack` answers, the inbox.
- The lattice reader: `deUrb`, the base probe, the revision cache, keens with yawn and every page mark, the vault fallback, `/k/` and `/t/`.
- Exit: traces 1, 2, 3, 6 and 9 replay byte for byte; the message vectors pass.

**M4. Serving** (2 days)
- The grubbery server: snaps, keeps, drops, waves, `changed`; scry serving; the keeper cap.
- Lattice's layout (`pub`).
- Exit: trace 5 replays; every built message clams against grubbery's molds in vere; trace 7 replays; saved meta.pub round-trips byte for byte with the JS's.

**M5. furum and lattice forums** (2 days)
- furum's boards, and the lattice forum reader: board, thread, post, refusal notes.
- Exit: traces 4 and 8 replay; the board vectors pass; every forum action we build clams against lattice's `action:ft` in vere.

**M5b. Rendering** (3 days)
- `urb-render`: `renderAs` and sniffing, gemtext, the Markdown subset, text, `rewriteHTML`, the CSP choice.
- Both outputs: the extension's parsed structures and the gateway's HTML.
- Fuzzing every renderer on random and hostile bodies: no panic, no output link the allowlist refuses, and time linear in the body's size.
- Exit: every rendering vector passes, hostile ones included; the fuzzers find nothing in a long run.

**M6. The wasm build and the JS shims** (3 days)
- `mesa.wasm` built from Rust, with `urb-render` in it; the shims keep the module interface, `gemtext.js` and `markdown.js` included.
- The row-journaling store stays exactly as glue.c's, so saved rows are the same format.
- Every WASI import Rust's standard library asks for is supplied by the shim, and nothing is silently stubbed.
- A wasm size budget: today's module is 1.7 MB, 600 KB gzipped. The new one must stay within 1.5 times that, or the plan is revisited. If `urb-render` would push it over, it ships as a module of its own.
- Exit: every trace replays through the wasm build; the extension's `npm test`, `npm run today`, `npm run urb`, `npm run pub` and the furum harness pass with the new files dropped into `lib/`; `lib/urb.js` and `reader.js` unchanged.

**M7. The native host and the command line** (3 days)
- UDP, timers, the roller and bootstrap, SQLite; comet mining; moon keyfiles and `fresh`.
- A command line with today's commands: read a page, serve pages, use furum.
- Exit: every trace replays through the native host; the Node command lines and the Rust one give the same output for the same fleet runs.

**M8. Replacing the Go gateway** (2 days)
- The HTTP gateway in Rust: same routes, headers and errors, with `urb-render` for rendering.
- It opens the Go gateway's existing state folder and keeps its comet and port.
- Exit: a side-by-side run serves the same bytes as the Go gateway for every page in a fixed list, from the fake fleet and read-only from live ships; `urb_test.go`'s and `reader_test.go`'s cases pass as vectors.

## M0's findings (2026-10-31)

M0's exit criterion passed: libmesa's suites give the same results built by mesa-sys as built by its own build.zig with zig 0.15.2. `core/mesa-sys/suites.sh` runs both, and CI runs them on every change.

| Build | Native | WASI |
|---|---|---|
| mesa-sys, debug | 12 of 12 | 11 of 12 |
| mesa-sys, release | 12 of 12 | 11 of 12 |
| mesa-sys, with zig Debug's UBSan | 12 of 12 | not run |
| zig, Debug | 12 of 12 | 11 of 12 |
| zig, ReleaseFast | 12 of 12 | 11 of 12 |

Under WASI store_test does not compile in either build (no mkdtemp in wasi-libc). The WASI tests link wasi-libc's emulated getpid, since they seed `rand()` with it.

**The toolchain is not zig's.** The explicit flags are build.zig's, but three things differ, and the suites are compared with them in place:

- The C compiler: mesa-sys uses the clang on PATH (23.1 here, Ubuntu 24.04's 18 in CI) and the wasi-sdk 34's clang 23 for wasm; zig 0.15.2 uses its own clang 20. Each suites run prints its compiler.
- The CPU: mesa-sys builds for baseline x86-64 (`-march=x86-64`), since that is what ships. zig defaults to the build machine's CPU, so suites.sh runs zig with `-Dcpu=baseline` for the comparison. On wasm both use lime1.
- Defaults neither build.zig nor build.rs sets: zig adds `-fstack-protector-strong` in Debug and `-fno-stack-protector` in ReleaseFast, where mesa-sys gets the clang's default; cc adds `-ffunction-sections -fdata-sections -fPIC -mno-omit-leaf-frame-pointer`.

So "the same results" means the same pass, fail or no-build per test, not the same object code. The traces are what prove the behaviour.

**No clock and no randomness.** The evidence:

- libmesa's built sources have no call to a clock, a timer or a random source. The only `rand`, `srand` and `getpid` calls are in `src/wire/pact.c` lines 1251 to 1517, inside `#ifdef PACT_TEST` (lines 1070 to 1532), which only pact_test builds. `vendor/` and the public headers have none. `src/host/host.c` has the libuv clock, but mesa-sys does not build it.
- The libc symbols `libmesa.a` imports, native and wasm, are memory, string and formatting functions, `abort`, `getenv`, and `open`, `close` and `flock` for the SQLite store's lock file. There is no `time`, `clock_gettime`, `gettimeofday`, `rand`, `getrandom` or `getentropy`.
- `getenv` is read only for `MESA_TRACE` (`eng_ames.c:279`, `eng_util.c:44`).
- urcrypt's ed25519 includes `ed25519_create_seed`, which reads `/dev/urandom`. Nothing in libmesa or urcrypt's wrappers references it, so it is never linked.
- SQLite imports `time`, `gettimeofday`, `localtime` and `getpid` for its own file handling, and `sqlite3_randomness` reads `/dev/urandom` for its temporary file names. It runs only in the native SQLite store, and none of it changes the rows the engine saves.

**A bindgen trap.** bindgen read `MESA_NONE`, which is `((mesa_noun)UINT64_MAX)`, as `i32 = -1`. That compiles and is wrong. build.rs now names the C type of every integer macro and stops the build on one it does not know. The same care goes into every later binding.

## M1's findings (2026-10-31)

**The recorder** is `wasm/trace.mjs`. open() takes two more hooks for it, `timers` and `fetch`, beside `now` and `entropy`. A trace is JSON lines: a head, then one step per input (`call`, `hear`, `tick`, `fetch`), each with its `now` and the entropy it drew, and the outputs it caused (`send`, `commit`, `saveKey`, `start`, `tclose`, `log`, `timer`, `untimer`, `fetch`, `result`, `event`). The clock is fixed for a step. Every step is a macrotask of its own and every hook is synchronous, so Node runs all of a step's microtasks before the next input and each output lands in its input's step.

Each timer has a name, which mesa.mjs gives it: `wake`, `point <ship>`, `roller <ship>`, `peek <wire>`, `poke <wire>`, `keen <path>` or `close`. Each `timer` output and each tick carries that kind and its `due`. A tick says which timer fires, so two timers due at once are two ticks, each named, in order, and a port fires exactly the named one at the tick's `now`. `timer` and `untimer` are the JS's own scheduling: a port drives its own timers by the ticks and compares the rest. The recorder waits out a timer Node fires early, so no tick comes before its `due`. The traces recorded before that were labelled by a replay (`replay.mjs --label`), and trace 5's two early ticks are marked `early`: a wake the JS finds not yet due and sets again. `node wasm/replay.mjs` replays every trace into a fresh engine and compares each step's outputs, byte for byte.

**The traces** are in `traces/`, recorded by `wasm/record.mjs` against the furum set's `m1c-traces` snapshot (`traces/fixtures.sh` builds it from `hus-lattice`). What the fleet taught:

- On the fake network an identity can be used once per snapshot. A fresh engine is breached at every open (`fake` forces it), and a ship that heard the identity before ignores it after. So each trace is its own fake moon of `~hus`, and re-recording resets the set first.
- A moon's sponsor on the fake network is a planet nobody runs, so a ship can reach us only once we have sent it something. Trace 5 reads `~hus` first, so its lattice can read us.
- **A ship that reaches us over old ames first never answers us over mesa.** Lattice's `/share-file` notifies the ship it grants. Sent to an identity that has never sent `~hus` anything, the notice goes over ames, and `~hus` keeps the identity as an ames peer (`%ax /peers/<ship>` is `%known`, and it has no `/chums/<ship>`). libmesa speaks only mesa, and `~hus` drops every mesa packet from that identity afterwards, unanswered, run after run. An identity that had reached `~hus` first is a mesa chum, and the notice reaches it: its read after the notice works, once `~hus` resends. This is not urb-fetch's to fix, since urb-fetch has no ames, and it reaches users: a lattice user who shares a page with a urb-fetch reader before the reader ever reads from them cuts that reader off. It is for arvo (a peer heard over mesa should move to mesa). The fixtures grant by group (`/share-group-save`), which sends nothing, and no trace records the share-file path until arvo is fixed.
- Lattice 81's `/unsub` culls its subscription grub and sends no `%drop`. Grubbery drops the keep when the next wave finds nobody watching (`+take-wave`'s self-heal), so trace 5's drop follows an edit: "the reader's drop" is grubbery's self-heal `%drop`, not one lattice sends. A keeper of a page that never changes is never dropped; that is for lattice.
- Two of trace 6's cases are constructed. A deleted page's old binding still answered our keen, so its keen that misses is of a revision list rewritten through grubbery's `/grubbery/api/over` to name an id the page was never grown at. Its poke with no `%pack` goes to `~fes`, which runs nothing, so it is a poke nobody answers; a poke acked whose `%pack` never comes is not recorded. Trace 9 rotates the salt through the same API, since lattice's `/grub-save` may not write it (its weir refuses the dart and the request hangs).
- Trace 10 is the roller: a moon off the fake network reads `~zod`, and the engine asks roller.urbit.org (read only) for `~hus`'s and `~zod`'s keys. The lookup's 30 s limit is an engine timer, `roller <ship>`, which aborts the fetch with `AbortSignal.timeout`'s error, but no trace has it fire: the roller can't be made to hang.

**The conformance vectors** are in `conformance/` (each file's `about` says what it holds). JS: `node conformance/run.mjs --ext <nisfeb-extension>`. Go: `go test -run TestConformance`. They are of nisfeb-extension 78c30bb, its main, which is 800ff22 plus the gemtext link fix M1a asked for. A mark of `""` is left out of `render-as`: urb.go reads it as no mark, mesa.mjs as text, and no page has it. The Go binding has no public way to make a noun, so the noun, message and decoder vectors run in the JS only; vere checks the kinds they cover in check.mjs.

The hostile vectors' errors are matched as text, so the Rust must give the JS's messages, word for word (`another ship's noun has over 200000 cells` and the rest), as the contract already asks of every error. check.mjs has vere check the vectors' nouns (by sham), maps and sets (as +malt and +silt build them) and messages (clammed against their molds).

**CI** runs check.mjs (with the molds), the JS tests, every trace's replay, the vectors in the JS, the extension and the Go, and `go test`. With `REQUIRE_ALL` set, as CI sets it, a test that would skip for want of a checkout fails instead. `rerecord.yml` records the traces again on a self-hosted runner labelled `fleet`, daily and by hand, and fails on a drift. `wasm/drift.mjs` compares the calls, results and events, with dates, ids and a mined comet's name masked, and what each trace put on the wire, by kind: every grubbery message by mark and tag, keens, yawns and served scries. No such runner is registered yet, so its first job fails with an error that says nothing was recorded: set the repository variable `FLEET_RUNNER` to `yes` once one is. `traces/rerecord.sh` runs it by hand.

## M2's findings (2026-10-31)

**The `urb` crate** is `core/urb`. `noun.rs` has the owned `Noun` (one libmesa reference, given up on drop) and the borrowed `NounRef<'a>`, which can't outlive its owner, nor, through `with_borrowed`, the hook that lent it (doc tests check both don't compile). Neither is `Send`. Every owned noun is counted per thread (`urb::alive()`), and every test ends with it at zero. `Noun::of` builds a `Value` as mesa.mjs's `noun()` does, maps and sets as its treap. `codec.rs` holds mesa.mjs's decoders with their caps and messages, `text.rs` uv and da, and `js.rs` what the JS did for free: TextDecoder's UTF-8 (a leading byte-order mark dropped), `Number()` of text, and a JS object's key order. `mesa-sys` now binds libmesa's private `sha.h` too, for `mesa_shaf`, which `+sham` is made of.

**The vectors.** `cargo test -p urb --test vectors` runs `nouns.json` and `hash.json` whole and the 53 cases of `decoders.json` and `hostile-nouns.json` that are mesa.mjs's own decoders: 255 cases, all passing, natively and under WASI. Each file's count is checked, so a case can't be skipped. Furum's and forum's reads (M5), the messages (M3, M4), deUrb (M3) and the renderers (M5b) wait for their milestones. libmesa's jam gives jam.mjs's bytes for every vector.

**The property tests** (`tests/props.rs`) draw from a printed seed (`URB_SEED`). Against vere: 600 random maps and sets, with keys of every kind we use and maps nested in them, up to 400 keys, hash as `+malt` and `+silt` build them, and 200 each of mug, uv and da agree. Against the JS (`tests/oracle.mjs`, which needs `wasm/build/mesa.wasm`): 2,000 random values, with short arrays and repeated Map keys, make mesa.mjs's jam or its TypeError; 2,000 byte strings read as a TextDecoder reads them; 4,000 strings give the same `Number()`. With `REQUIRE_ALL` set, a missing vere or node fails.

What they and the vectors taught:

- **Port the JS's accidents too.** A cord's text drops a leading byte-order mark (TextDecoder), so `﻿o` reads as the tag `o`. `$json`'s `%n` is `Number()` of the text, so `0x10` is 16 and ` 12 ` is 12. An object's keys come out with array indices first. A Map with two equal array keys has both in its treap. Each is ported, and checked.
- **serde_json's default float parsing is not exact,** so the tests compare numbers with `float_roundtrip` on. Node's `Number()` rounds hex to nearest, ties to even, as `js.rs` does.
- **libmesa's `mesa_eq` walks a noun as a tree,** so comparing two equal copies of a shared noun takes as long as its tree is big, and jam's dedup compares with it. The first fuzz run found this (the cue target's round trip). It is why every noun from another ship is measured before anything walks it, as the JS does; `tangText` measures nothing itself, so its caller measures, as `on_poke` does.

**Fuzzing.** `core/urb/fuzz` has a `cargo fuzz` target for every decoder: cue, measure, list, entries, segments (with pathOf and knot), json (with `Number()` and TextDecoder), tang, tangText, fileOf, ballOf and ship. An input is a jam or a program for a small stack machine that makes shared cells vast as trees, the tags the decoders look for, long lists and big atoms. Each checks it never panics, that every error is a hostile-noun error, and that no noun is left alive. `fuzz/seed.mjs` seeds them from the vectors. Every target ran for 21 minutes twice, with AddressSanitizer and without, and found nothing past the cue round trip above:

| Target | Runs, with ASan | Runs, without |
|---|---|---|
| cue | 8,226,052 | 28,053,135 |
| measure | 69,675 | 82,335 |
| list | 22,853 | 88,537 |
| entries | 44,250 | 147,460 |
| segments | 35,978 | 162,538 |
| json | 96,835 | 353,536 |
| tang | 54,766 | 190,187 |
| tangText | 42,975 | 199,334 |
| fileOf | 12,647 | 137,576 |
| ballOf | 84,447 | 202,564 |
| ship | 65,979 | 166,959 |

A run is slow (tens to hundreds a second) because a hostile input makes a decoder walk to its 200,000 cap before it refuses. The slowest is the hostile ballOf vector of over 200,000 trees and files: 0.8 s in a release build, about 6 s under ASan, since each file's path is copied, as the JS's `[...at, name]` copies it. It is bounded, but on a loaded machine it passed libFuzzer's 10 s limit, so the targets run with a 30 s one. CI runs each for a minute.

**What M2's review found.** Three nouns passed `measure` and still blew up a decoder, in the hardened JS as well as in the port:
- ballOf's directory loop did no counted work for an entry that isn't a tree (259 s);
- a tank's separator, printed between each two kids, multiplied text (a 150 KB jam printed 3.6 GB);
- one big atom backref'd at many places was decoded at each of them (a 2 MB jam read as 4 GB).

`measure` bounded cells, but not the work or the output that walking shared structure produces. The fix went into the JS and the Go on main first (PR #5), and then here:

- `measure` counts atom bytes as a tree too, under `MAX_BYTES` (64 MiB, libmesa's largest message), from a `Budget` that the whole decode shares. The tank printer spends each separator it prints after the first. ballOf counts every directory entry, and spends each file's path and noun at each place the file appears.
- `measure` returns a `Measured`. Only a `Measured` or an owned `Noun` can be jammed, hashed or compared: `NounRef` has no `==`, `jam` or `sham`, since libmesa's versions walk a noun as a tree. `tang_text` takes a `Measured` and the `Budget` it was measured from, so a %pack is measured and printed from one. A peer's noun used as a `Value::Map` key must come from one (`From<Measured> for Value`).
- `Debug` prints at most 64 cells, and an atom's low 32 bytes. `Number()` of a `0x` literal folds its digits as it goes, instead of keeping a bit for each one.
- The hostile vectors carry their whole message, and every runner matches it exactly. Eight new vectors cover the three blowups.
- The fuzz targets assert that a decode's output stays under `OUT_MAX` (three times `MAX_BYTES`, plus a little per cell). The stack machine can now build the shapes: a 2 MiB atom, a 10,000-long tape, a map of many names for one ject, and a tree ject.
- One case is bounded only by the product of the caps. ballOf's paths in Rust are a `Vec<String>` per file, so 200,000 files 65 empty segments deep cost about 300 MB of String headers; the JS's arrays cost about 100 MB.

## M3's findings (2026-10-31)

M3's exit criteria passed: traces 1, 2, 3, 6 and 9 replay into the Rust core byte for byte, natively and under WASI, and `messages.json` (39 cases) and `deurb.json` (60) pass. Traces 7 (the `%want` leg) and 10 (the roller) replay too. 4, 5 and 8 wait at their first furum, pub or forum call, which are M4's and M5's.

**The core** is `core/urb/src/core.rs`. `Core::step(now, input)` takes one input (a request with an id, a datagram, a tick, a fetch's answer) and gives its outputs (datagrams, rows to commit, the saved key, the transport's start and close, fetches, log lines, each request's result, and inbox pokes). Entropy comes from a function the host gives `Core::new`. Its timers are named as mesa.mjs names them, and a tick names the timer it fires and its due, so the replay fires exactly the timer the trace's tick fired; `deadline()` is the earliest, and `due(now)` the one a host fires next. The requests are trace.mjs's calls, `open` (comet mining included, through a fetch of the comet stars), `read`, `close`, `peek` and `poke`, with their results and errors, word for word, even trace.mjs's own (`trace: one engine at a time`, `Cannot read properties of null (reading 'read')`). `engine.rs` is glue.c: the row-journaling store, written as glue.c writes it, and the hooks. Each hook is an `extern "C"` trampoline that runs its body under `catch_unwind` and answers the hook's default, logging an error as `guarded()` does. Hooks re-enter the engine (a %pack is poked from inside the plea hook, a flow corked from inside done), so the core's state is in `RefCell`s that are never borrowed across an engine call.

**The order of a step's outputs is the JS's job queue.** A step's outputs are in a trace in the order Node's microtasks made them: a commit is emitted one job after the engine call that made it, its datagrams two after, and a read's result as many jobs after the answer that settles it as the awaits between them. So trace 3 has a commit, a result and then a datagram in one step. Rather than count each depth by hand, `rt.rs` keeps ECMAScript's queue: promises that run their reactions as jobs, adoption of a promise in two jobs (NewPromiseResolveThenableJob), `finally` as the spec has it, and async functions whose awaits resume a job after what they await settles. Each of mesa.mjs's functions is ported with its awaits where the JS has them, so `return readPrivate(...)` in an async function is two jobs slower than `return await`, as in the JS. An adoption's reaction must resolve with fresh resolving functions: the first replay of trace 3 lost a `/k/` read's result because the promise was already marked resolved when it was handed the other.

**libmesa built natively gives mesa.wasm's bytes.** The traces were recorded with mesa.wasm (wasm32, zig's clang 20). Replayed into libmesa built by mesa-sys for x86-64 and for wasm32-wasip1, every datagram and every commit is the same, byte for byte. The engine is deterministic across both, as M0 expected.

**The replay bites.** After each step the core's timers must be the ones the JS had live (rebuilt from the trace's `timer`, `untimer` and ticks, all cleared when close gives its result), by kind and due, and `deadline()` their earliest. Each of these makes a required trace fail: a settled peek's timer left set, a stray 0 ms timer, the keen, peek or close timeout a millisecond longer, the revision cache's 60 s cut to 1 s, or the wake's `>=` made `>`.

**What it taught:**

- mesa.mjs calls `X.deadline()` after a refused open, when glue.c's engine is NULL. Under wasm that read low memory: zeroed, it gave no deadline, and with 0xff there it traps. By rule 2 it was fixed on main first (PR #7): every glue.c export that takes the engine checks for one, and `store_load` bounds each row by what is left of its buffer, since a length near 2^32 wrapped a 32-bit `size_t` past its checks. `Journal::load` had the same wrap and checks every sum too. The core never asks a closed engine anything.
- The roller's 30 s limit aborts the real fetch through its signal, and trace.mjs's fetch ignores the signal: a lookup that never answers would hang in a replay. In the core the `roller <ship>` timer does nothing, as in a replay, so the native host (M7) must bound its own fetch.
- A request's state holds the reaction that waits on it, and that reaction holds the engine: the core breaks the cycles when it closes the engine.
- We serve no files yet, so a `%peek` from another ship is answered with an empty `%snap` (with 16 bytes of entropy for its id, as the JS draws), a `%keep` with an empty wave, and a scry with nothing. M4 serves.
- On wasm32-wasip1 Rust aborts on a panic, so `catch_unwind` guards the native build only; under WASI a panic stops the module, as the plan's risks say.

**Why not a simpler order.** Comparing each kind of output as its own sequence, with commits before the datagrams they release as an invariant, would let the core emit at once and drop rt.rs. It can't replay the traces byte for byte (trace 3's commit, result, datagram), so M3 keeps the JS's queue. Whether to relax the contract is for before M4 and M7.

**The props flake** (vere gave 1,129 answers for 1,200 on CI) was the test's escape stripper: it skipped from an ESC to the next `m`, so a CSI ending in another letter ate the hex answers after it. It now strips a CSI to its final byte, and a short count writes vere's whole output to a file.

**Fuzzing.** Two new targets: `de_urb`, any text through deUrb (only its errors, never a panic), and `hooks`, a core with a fake engine open and a peek, a poke and a keen waiting, handed another ship's `%grubbery-transfer` on each wire and on `/want` (after a `%snap` with no data), a `%grubbery-load`, a nack and a keen's answer, each a fuzzer's noun. The hooks must never panic, every noun must be let go, and what they give must stay under `OUT_MAX`; the core's timers then run past the 30 s timeouts. `seed.mjs` seeds both. A brief run found nothing: hooks ran 26,423 inputs in 15 minutes with AddressSanitizer and LeakSanitizer and 37,012 in 5 without (each opens an engine), and de_urb 11,450,705 in 5 minutes with them and 30,634,171 in 3 without. CI runs each for a minute, and the replay.

## M4's findings (2026-10-31)

M4's exit criteria passed. Trace 5 replays into the Rust core byte for byte, timers included (its two early wakes too), natively and under WASI, debug and release, and so do 1, 2, 3, 6, 7, 9 and 10, which the replay now requires. Every message the core sends clams against grubbery's molds in vere. Saved meta.pub round-trips byte for byte with the JS's.

**The server** is mesa.mjs's, in `core.rs`. A `%peek` of a file we serve is answered with its `%snap` and silo inline, a `%keep` with its first wave and kept, even of a page not yet published (up to 256 keeps, `MAX_KEEPERS`, and 64 a ship, `PER_SHIP`; a keep held already is never evicted, and one unchanged is not saved again), a `%drop` forgets the keep, and `grub.changed` sends each keeper of a file its new wave, in one transaction. A `%keen` of our namespace is answered by `scry.serve`. `grub.serve`'s three functions are closures (`Files`, `Scries`, `OnKeepers`) the engine's state holds, so the shims of M6 can hand in the JS's own. `publish.rs` is pub.mjs: `pageKey`, the index, revs and vault files, the page scry, `put` and `remove`. The calls are trace.mjs's `pub`, `pub.put`, `pub.remove` and `pub.pages`, and each save is an event, `pub.save`.

**meta.pub is kept as JS values, not as a struct.** `publish.rs` holds the saved `{pages, keepers}` as `js::Json`, in the order the JS keeps an object's keys, so whatever a host saved is saved again as it was: a page from before marks keeps having none, a removed page has none either, and a field the JS never wrote survives. `js::stringify` is JSON.stringify. `tests/publish.rs` publishes from 16 saved states (trace 5's saves, an empty one, and older and odder shapes: keys pageKey never makes, revs and times that are fractions, text or missing) in pub.mjs on mesa.wasm (`tests/pub-oracle.mjs`) and in the core, runs 120 ops on each (puts and removes with Unicode bodies and names, and other ships' peeks, keeps and drops, up to a ship's share), and requires the same results, the same 1,454 saves as JSON text, the same pokes by sham and the same log lines, byte for byte.

**Port the JS's accidents, again.** put's rev is `(rev ?? 0) + 1` and remove's is `rev + 1`, as JS adds (`js::add`: a saved rev of `"2"` becomes `"21"`), so a saved page with no rev comes back up at 1 and goes down at NaN. A rev or time that is not a whole number fails where the JS's `BigInt()` and `da()` fail, with their RangeError, TypeError or SyntaxError word for word (`js::bigint`, `msg::da_of`), so `msg::File` keeps the rev and time as the JS values they are. The index reads each page's hash as `BigInt('0x' + hash)`, which throws for a page saved with none: a peek of the index logs the error and is not answered, and every put fails after its save. The index's key is the page's key less its first UTF-16 unit, as `k.slice(1)`, so a saved key that is empty or starts with a character past the BMP lists, and doesn't panic, as the first port did (the review's must-fix). A saved keeper whose ship is not one makes every put of its page fail with `not a ship: …`, after the save. Each needs a state pub.mjs never writes.

**Two fixes went to main first** (rule 2, PR #8), from the review: one ship could hold all 256 keeps, saved across restarts, and every repeated keep saved the whole of meta.pub again. Now each ship holds at most 64 of the 256, and an unchanged keep is not saved. The PR's first try refused keeps of pages with nothing in them, which broke lattice's reader: it keeps a page's vault file before the page exists and again on a reload after a removal (app.hoon 462-503), so a follow made then went silent. Such keeps are taken again. Both are ported, and `tests/publish.rs` checks a keep before publication, a keep again after a removal, and one ship's share, against the JS.

**The molds.** `capture_pokes` keeps the mark and jam of every poke the core makes on its thread. The replay test clams each distinct one, from every required trace and from a serving session of peeks, keeps, edits and a drop from another ship, against `load`, `transfer` and `intake` in check.mjs's copy of grubbery's molds, by `(sham ;;(mold (cue …)))` against the Rust sham. 50 distinct messages fit: 20 load `%peek`, 1 load `%poke`, 17 transfer `%snap`, 1 `%pack`, 1 `%want` and 10 intake waves. A load with a deep flag of 2 and a `%snap` whose pace is `%fir` must be refused, and the same `%snap` with `%firm` must fit.

**The replay bites** for serving too. Each of these fails trace 5: no room for a keep, the page scry's id cut one short, the page hash in upper case, the list's version one higher, and a keep that does not save.

**Fuzzing.** A new target, `serve`: a core publishing pages, one taken down and one kept by a saved keep, hears a `%grubbery-load` whose whole noun, wire, dest or folder is the fuzzer's, for each of `%peek`, `%keep` and `%drop`, or a scry of the fuzzer's bytes as a path; then an edit waves every keeper and the timers run. Its pick 5 publishes from a saved page whose key is the fuzzer's text, seeded with the empty and multibyte keys, with a whole or fractional rev, and peeks the index and the list. No hook may panic, every noun must be let go, the keeps must stay under the cap, and what it gives must stay under `OUT_MAX`. `seed.mjs` seeds it. A brief run found nothing: serve ran 41,877 inputs in 12 minutes with AddressSanitizer and LeakSanitizer and 60,931 in 5 without, and hooks 21,747 in 3 without. After the review's fixes, with pick 5, serve ran 12,451 in 6 minutes with them and 23,904 in 3 without, and found nothing. CI runs it for a minute with the rest.

## M5's findings (2026-10-31)

M5's exit criteria passed. Traces 4 (furum) and 8 (a lattice forum) replay into the Rust core byte for byte, timers included, natively and under WASI, debug and release, and the replay now requires all ten traces. The 25 board vectors pass (19 in decoders.json, 6 in hostile-nouns.json). Every forum action the core builds clams against lattice 81's `action:ft` in vere.

**What reads** is `forum.rs`, pure and with no IO: furum's next post, posts, comments, votes and notes (`forum::furum`), and lattice's board grubs read into load:fb's board (`board::Acc::read`, `assemble`, `Board::thread`) and its refusal notes (`forum::board`). Each decoder takes a `Measured` noun. codec now hands a peek's conv a measured noun: `file_in` reads a file out of a silo measured whole, and `ball_of` gives each file as it measured it from the peek's budget. One @da function serves both apps, `((d >> 64) - epoch) * 1000` plus the fraction's ms, null past 8.64e15 ms either way; furum.mjs's floats are exact wherever a Date is valid, so the two agree. forum.mjs's ids are exact (up to 2^53 - 1 a number, past it a BigInt). `js::Js` is the value an app gives back, with trace.mjs's `$big` and `$date`; `js::iso` is toISOString, years past 9999 and before 0 included, and `js::int_number` is Number() of a BigInt, ties to even. `js::Object` is generic now, so an app's objects keep JS's key order (a board's `comments` is keyed by post id, which sorts as array indices).

**The calls** are `core/apps.rs`: `Request::Furum` and `Request::Forum`, with `FurumCall` and `ForumCall`, each ported with its awaits where the JS has them. A peek's conv may be an app's own (`Conv2::File`, `Conv2::Dir`), which runs during the hook as the JS's does, so a grub forum.mjs can't read fails the peek with its message. grub.onInbox is a `Listener` (the plain `Output::Inbox`, furum's, or forum's for its host), and a note an app hears is an `Output::Note` (`furum.note`, `forum.note`). A forum's base is kept per host for the reader's life, as trace.mjs's `forumOf` keeps the reader, and forgotten at close.

**Port the JS's accidents, again.**

- furum.mjs's ids are `Number()`, so past 2^53 they round. forum.mjs's are exact.
- furum's newComment sent a parent of 0 as none (`some` is truthiness), but furum's comment ids start at 0, so a reply to a post's first comment went out as a top-level comment. By rule 2 this was fixed in the JS first, in https://github.com/nisfeb/urb-fetch/pull/9: only an absent parent (null or undefined) is none. `core/apps.rs`'s `some_parent` now matches the fixed JS, with a unit test; no trace calls newComment with a parent, so none changed. A post id that is a fraction fails with BigInt()'s RangeError where noun() runs, inside the poke's transaction, after its timer is set; `grub_poke_with` takes the value as a Result for that.
- furum's posts() of no board is a plain `Error`, not an `UrbError`.
- forum's board() and thread() check the board name only once the host's base has answered (`${await hostBase()}/forum/boards/${boardName(name)}`), so a bad name costs a base peek first. Its actions check the name, the title, the body and each id at once, before the base is asked.
- A conv that gives undefined finishes a %want's peek, since the JS checks `v !== null`. The core checked `is_null()`, which counts undefined too; it now checks null only. No conv the core runs gives `Val::Undef` today (furum's comments of a post its bucket lacks is `Val::Any(None)`, which finished the peek before the change too), so this is unobservable and only removes a trap for a future conv.

**The replay bites.** Each of these fails a trace: the base's 8 s wait made 8,001 ms, the base never cached, and furum's posts sorted oldest first.

**The molds.** `every_forum_action_clams_against_lattices_action_ft` (tests/replay.rs) captures every `[/lattice %forum-action]` poke: three from the traces (trace 8's two posts and trace 6's upvote to ~fes) and ten that a session of forum calls builds (posts with and without a url and body, Unicode and 2,000 bytes, comments with a parent, an id past 2^53 with a text parent, an upvote, downvote and unvote of a post and of a comment). Each is clammed as `(sham ;;(action:ft (cue …)))` against check.mjs's copy of lattice 81's types and compared with the Rust sham: 13 of 13 fit. Five shapes no action has must be refused. Renaming `remove-vote` fails it. With traces 4 and 8 required, grubbery's mold check now covers 78 distinct messages.

**Fuzzing.** Two new targets. `forum` runs every decoder in forum.rs on the fuzzer's nouns: furum's buckets and notes, lattice's notes, and two board grubs at places the input picks, read into one board, which is then assembled, with its JS and a thread. Errors must be hostile-noun errors or forum.mjs's own. `apps` runs furum's and forum's calls on the core, answered by a host whose grubs hold the fuzzer's nouns in well-formed %snaps (a file, or a directory of files under trees), so the apps' convs read them during the hook; it also pokes notes into our inbox for each app's onNote, then runs the timers out. In both, nothing may panic, every noun must be let go, and nothing given back may pass `OUT_MAX`. `seed.mjs` seeds both from the board vectors. In a one-minute run `apps` never assembled a whole board from two grubs at random places, so its pick 7 answers a thread with a whole board, its two nouns split as [card conf] and [posts threads votes], and `seed.mjs` seeds that pick with each whole board of the forum.board and forum.thread vectors, for each post id the target can pick. Each ran for 11 minutes with AddressSanitizer and found nothing: forum 56,930 inputs and apps 46,879. Its first run found a gap in the target's own check, not in the port: a thread of a post the board lacks is forum.mjs's `no post …` error, which the check had not listed. CI runs each for a minute with the rest.

**Running the tests by hand.** A cargo that is not rustup's (a distribution's `/usr/bin/cargo`) ignores `rust-toolchain.toml`: native tests pass on it, and WASI fails with "can't find crate for core". `core/ci/test.sh` runs fmt, clippy and cargo test natively and under WASI as CI does, with rustup's `~/.cargo/bin` first on PATH, and stops at once, naming the cargo it found, if rustc is not the pinned one. Extra arguments go to each cargo test (`core/ci/test.sh --release`). wasmtime must be on PATH (`core/ci/tools.sh`).

## M5b's findings (2026-11-01)

M5b's exit criteria passed. Every rendering vector passes in Rust, hostile ones included: 10,417 cases in seven files, each file's count checked. Each renderer was fuzzed for 11 minutes with nothing found, and its time is asserted linear in the body's size. All ten traces still replay, and fmt, clippy (native and wasm32-wasip1) and the native and WASI tests pass through `core/ci/test.sh`.

**What renders** is `core/urb-render`, pure Rust, with no dependencies and nothing from `urb`:

- `ext` is the extension's `lib/gemtext.js` and `lib/markdown.js` (nisfeb-extension 78c30bb): `gemtext`, `link_target`, `link_href`, `reader_path`, `spans`, `markdown`, `md_nodes`, `safe_link` and `MD_MAX`, with the same blocks and spans as enums (`Gem`, `Block`, `Span`) and mdNodes' nodes as a DOM would hold them (`Node`). Text is a JS string, so lengths are UTF-16 units and whitespace is JavaScript's (`js`). linkTarget's `new URL(href, base).pathname` is `whatwg.rs`, the part of a WHATWG parser an href that linkTarget lets through can reach.
- `gateway` is `render.go` and `urb.go`, byte for byte: `render_gemtext`, `render_markdown`, `render_text`, `rewrite_html`, `page`, `page_link`, `gateway_url`, `safe_link`, `looks_html`, `render_as`, and `render`, main.go's choice of renderer and Content-Security-Policy (`SANDBOX` for a publisher's HTML, `OWN` for the rest, `RAW` for `?raw`). A Go string is bytes and the Go reader hands a body over as it came, so the gateway's renderers take and give bytes: a byte that is not UTF-8 reads as Go reads it (U+FFFD, one byte, one UTF-16 unit) and is written back as it was.
- `render_as` and `looks_html` at the top are mesa.mjs's. A mark of `""` is text there and no mark in `gateway::render_as`, as the two sources have it.
- `ship` holds both ship checks: `is_ship` is the extension's shape check, and `parses` is libmesa's `u3_ship_of_string`, real syllables in their canonical rendering only. `parses` renders the chunks it read and compares, since `+fein` and `+fynd` are inverses; urb's `ship_names` test holds it to `mesa_ship_parse` on 20,000 names.

**The vectors.** `cargo test -p urb-render --test vectors` runs render-as.json (192), render-ext.json (334), render-go.json (347), hostile-render.json (224) and three new files, render-ext-random.json (3,000), render-go-random.json (3,800) and render-go-bytes.json (2,520). render-as and hostile-render are both sources' rules, so each of their cases runs on both renderers. `conformance/random.mjs` makes 400 bodies and 600 link targets from a fixed seed (`inputs/random.json`), of the characters the rules turn on; gen.mjs writes the extension's readings and `UPDATE_VECTORS=1 go test` the Go's, and the JS and the Go run both, as they run the rest. Mutating `'`'s escape fails render-go, and dropping `'` from encodeURIComponent's set fails render-ext-random, which render-ext alone did not catch.

render-go-random also holds renderText and rewriteHTML on those bodies, and rewriteHTML on 1,000 bodies of random attributes the Go makes itself (`randomHTML` in conformance_test.go: its three words in any case and with ſ, near misses, whitespace RE2's `\s` takes and some it doesn't, either quote or none, and urb:// links to names in its class and outside it). rewriteHTML is the one transform a publisher's sandboxed HTML goes through, and render-go's 65 cases alone caught a dropped `'` quote in only one case. Now dropping the `'` quote fails 58 cases there (and 37 in render-go-bytes), dropping `action` 32, dropping `\f` from `\s` 29 (render-go caught none), and dropping `-` from the name's class 67 (render-go caught none).

render-go-bytes.json carries the gateway's reading of bytes that are not UTF-8, which a JSON string cannot hold: every argument and every output is hex. The Go makes 320 random bodies of lone leads and continuations, truncated sequences, surrogates, overlongs and 0xFF among the renderers' marks (`randomBytes`), and 300 such links, and writes renderGemtext, renderMarkdown and renderText as whole pages, rewriteHTML, looksHtml, renderAs, safeLink and pageLink on them. `go test -run TestConformance` and the Rust's `every_bytes_vector` run it, in CI too.

**The vectors' document lost text.** conformance/lib.mjs built its elements with `Object.assign`, which calls each getter once and copies the value, so its `textContent`, `href` and `rel` setters never ran: every pre, code, strong, em and link in the mdNodes vectors had no text, and no link had an href. It now defines them as accessors, and render-ext.json was written again (only its mdNodes cases changed). The fix is on rust-core only, since conformance/ is.

**Differences between the JS and the Go** (rule 2):

1. **Go's `(?i)` folds `ſ` (U+017F) into `s`** and the Kelvin sign into `k`, so the gateway's safeLink, scheme and webLink took `httpſ://e.example/x` for the web and rendered `<a href="httpſ://e.example/x" rel="noopener noreferrer">`, where the extension's ASCII-only `/i` leaves it as text. A browser reads that href as a path on the ship's own origin, so nothing escaped. Fixed for main in https://github.com/nisfeb/urb-fetch/pull/10 (open, not merged: M5b's sign-off and rust-core's merge wait on it), with `TestSafeLinkAsTheExtension`, and cherry-picked onto rust-core (fc9e22e) so that the Go, the vectors and the Rust agree. The Rust ports the fixed Go. hostile.mjs adds `httpſ://` twice.
2. **hostLink's `\s` was RE2's ASCII class**, so the gateway's safeLink let `"\u00a0//e.example"` through. pageLink refused it after its trim, so no HTML changed. Fixed in the same PR (hostLink uses `jsS`), and hostile.mjs adds three such links.
3. **Not settled: an urb:// link to a name that is not a real ship.** The extension's linkTarget checks the name's shape (`isShip`), and the gateway's pageLink parses it (`mesa.ParseShip`). So `urb://~abc/x`, `urb://~dozzod/x` and `urb://~doznec/x` are a link to the reader in the extension, which then fails to read them, and text in the gateway. The fix belongs in nisfeb-extension (its linkTarget checking a canonical @p, as `ship::parses` does), which this milestone may not touch, so it is proposed here and each renderer keeps its own check. The proposal (an `isPatp` with the gateway's rule, checked against `mesa.ParseShip` on 20,000 random names) is in https://github.com/nisfeb/nisfeb-extension/pull/8, a draft, since the code is in the extension's lib/. tests/two.rs and the `two` fuzz target allow exactly this difference, and no other.
4. `rewriteHTML`'s `(?i:href|src|action)` also folds `ſrc`. It has no counterpart in the extension and rewrites an attribute no browser reads, so it is ported as it is.

**A bug in the extension: ITEM is quadratic.** markdown.js's `ITEM`, `/^\s*(?:[-*+]|\d+[.)])\s+(.*)$/`, backtracks over the space after the marker once per character when what follows holds a line terminator: `"- " + " ".repeat(n) + "a\r"` takes 1.5 s in node at n = 40,000, so a 256 KB line would hold the reader's tab for about a minute. The Go (RE2) is linear, and the Rust reads it in one pass with the same result. The proposed fix, for nisfeb-extension, matches the item by hand as headingText does; it gives ITEM's result on 300,000 random lines and reads a 256 KB line in 1 ms:

```js
function itemText(line) {
  const s = line.trimStart()
  const m = /^(?:[-*+]|\d+[.)])/.exec(s)
  if (!m) return null
  const after = s.slice(m[0].length)
  const text = after.trimStart()
  if (text.length === after.length || /[\n\r\u2028\u2029]/.test(text)) return null
  return text
}
```

The body goes into hostile.mjs once the extension has the fix (the shared vector would fail the JS's 2,000 ms today). Until then it is in urb-render's tests/linear.rs.

**Tracked before M6 (rule 2):** both extension fixes are proposed in https://github.com/nisfeb/nisfeb-extension/pull/8, a draft holding the tested itemText and isPatp, with todo tests on lib/ that pass once lib/ takes them. It has to land in the extension before M6 replaces markdown.js with the Rust, or the JS reference stays quadratic and the hostile ITEM vector never reaches hostile.mjs. Owner: sneagan.

**Each `[` costs the same.** A `[` is followed by a scan of up to 256 units for its `]` and up to 512 more for its `)`, as the regex's bounds have it. That is linear but slow: 100,000 `[` took 2.5 s in a debug build, over the vectors' 2,000 ms. So each renderer reads a line with a `[` in it once, for each byte's next `]`, next `)` or space, and UTF-16 offset, and each `[` then costs a lookup. The results are the same.

**Linear time, asserted.** Each renderer counts its steps (the `cost` feature, on in urb-render's own tests and the fuzzers, and nothing otherwise). tests/linear.rs renders 31 bodies made to be slow, hostile.mjs's and those above: at most 64 steps a byte, 8 times the body at most 9 times the steps, and the clock within 24 times (quadratic is 64). An injected copy of ITEM's backtracking fails both.

**The two renderers held together.** tests/two.rs reads 60,000 random bodies and 200,000 random links with both and wants the same blocks, spans, sniff, safeLink and link, as render_parity_test.go holds the Go to the JS; the only difference it allows is item 3.

**Fuzzing.** Three targets in `core/urb-render/fuzz`, seeded by seed.mjs from the rendering vectors. `ext` reads a body with the extension's renderers, and each line as a link target; `gateway` renders any bytes as every mark; `two` holds the two together. None may panic, give a link the allowlist refuses (an href in the extension's nodes or the gateway's HTML that is not a reader page, http(s), mail or gemini, or that holds a control character or space), let a tag of the page's through, or take more than 64 steps a byte. Each ran for 11 minutes with AddressSanitizer, inputs up to 20,000 bytes, and found nothing: ext 322,703 inputs, gateway 152,566 and two 467,509. Injecting a scheme-blind pageLink is caught within seconds. CI runs each for a minute.

## M6's findings (2026-11-01)

M6's exit criteria passed. Every trace replays through the wasm build, and the extension's `npm test`, `npm run today`, `npm run urb` (in its fleet mode, see below), `npm run pub` and the furum harness pass with the new files dropped into the `lib/` of a throwaway clone of nisfeb-extension, with `lib/urb.js` and `reader.js` unchanged (their sha256 checked before and after). The new modules are 74% of today's size, and 77% gzipped. nisfeb-extension itself was not changed.

**What was built:**

- `core/urb-wasm` is the new `mesa.wasm`: the sans-IO core behind a few plain exports. A core's inputs and outputs cross as JSON in trace.mjs's encoding (`wire.rs`), so a trace's steps go in as they were recorded, and an output carries a little more than a trace does (an error's class, an inbox poke's grub, mark and jam). Entropy comes in through one import, `host.entropy`. It also exports keyfileInfo, fakeKeyfile, shipName, sham and mug.
- `core/render-wasm` is `render.wasm`: urb-render's extension half (gemtext, spans, markdown, the two calls of mdNodes, safeLink, linkTarget, linkHref, readerPath), renderAs and looksHtml, and deUrb, uv, da and pageKey.
- `shim/` holds the shims: `mesa.mjs`, `pub.mjs`, `furum.mjs` and `forum.mjs` on mesa.wasm, `gemtext.js` and `markdown.js` on render.wasm, and `render.js` and `wasi.js`. The host part stays JS, as the architecture has it: the transport (relayTransport, verbatim), the store, timers, fetch, and readRows and writeRows. `shim/build.sh` builds both modules with the `wasm` profile (release without symbol names), checks the size budget, and stages a `lib/`.
- Tests: `shim/replay.mjs` (the traces through the wasm build), `shim/rows.mjs` (saved rows across the two modules), `shim/fail.mjs` (the shims' failure paths), and unit tests of `wire.rs`. CI's core job runs them, and nisfeb-extension 78c30bb's `npm test` and renderer vectors on the shims.

**Every trace through the wasm build.** `node shim/replay.mjs` feeds each trace's steps to the core in `shim/build/mesa.wasm`, in node's V8 with the shim's own WASI imports, and compares each step's outputs, timers and entropy with the trace's, as replay.rs does on the native and WASI builds. Each runs twice, with WASI's random_get pinned to all 0x00 and then all 0xff bytes, so a hash seed that reached an output would fail every run, not flake. All ten pass both ways. A trace with one commit byte changed fails at its first step.

**Two modules, not one.** The plan put urb-render inside mesa.wasm. It ships as a module of its own, for a reason the plan didn't foresee rather than for size: gemtext.js, markdown.js and mesa.mjs's renderAs, deUrb, uv and da are synchronous, and the extension's service worker imports gemtext.js and mesa.mjs. A module service worker may not use a top-level await, and mesa.wasm comes as bytes to `open()`, so nothing would be there to render with. render.js compiles render.wasm synchronously from bytes it carries as base64 (`render-wasm.js`, built by build.sh, not committed). At 237 KB it is far under the 8 MB a browser lets a page compile synchronously on its main thread. It compiles on the first call, not at import, so a browser without wasm (a policy, a site setting, a CSP regression) fails that call and each after with `render.wasm failed to start: …`, and the service worker, which imports a renderer for every feature, still loads.

| Module | Bytes | Gzipped |
|---|---|---|
| today's mesa.wasm (glue.c, zig) | 1,698,062 | 598,362 |
| mesa.wasm (urb-wasm) | 1,015,334 | 368,278 |
| render.wasm | 237,385 | 91,655 |
| the two together | 1,252,719 | 459,933 |
| the budget, 1.5 times today's | 2,547,093 | 897,543 |
| mesa.wasm with render.wasm linked in (feature `render`, measured only) | 1,121,902 | 401,444 |

So one module would also have fit, with room. Why the Rust module is smaller than glue.c's was not looked into.

**The row-journaling store is glue.c's.** The core's store is `engine.rs`'s Journal, ported from glue.c in M3, and the shim hands its commits to the host's store as glue.c's `[klen:4][key][vlen:4][val]`, with readRows and writeRows unchanged. `node shim/rows.mjs` proves it both ways. It runs today's mesa.wasm (through wasm/mesa.mjs) and the Rust one (through the shim) side by side, on one virtual clock and one entropy, each over a store kept as the extension keeps IndexedDB: a commit's rows put or deleted by key, and a load handing every row back in the keys' binary order. A session opens a fake moon of ~zod, reads a page nobody answers, runs 45 virtual seconds and closes.

1. From empty, both send, commit and answer the same, byte for byte, and save the same 9 rows.
2. Today's module on the 9 rows the Rust one saved, and the Rust one on today's: the same again.
3. Crossed once more, on the 12 rows each saved in 2: the same again.

Sessions 2 and 3 send what a start from empty would not, so the rows loaded were used. Off the fake network, both refuse a moon that ran from an empty store, word for word. Dropping one row from the Rust side's load fails seven checks.

The sessions commit no deletes, so rows.mjs also checks the edges. Each module's readRows and writeRows, on rows with deleted ones, empty keys and values and an 8 MB value, write the same bytes and read back what the other wrote. Trace 1's commits, three deletes among them, read into a store by each, keep the same rows. And a session of each module on raw loads (empty, torn by 1 and 3 bytes, a torn header, a torn key, PR #7's near-2^32 key and value lengths, an empty key and value, a deleted row, an 8 MB row) gives the same results, datagrams and commits, the torn and over-bound loads refused with `store_load failed: mesa error -1`. readRows ignoring the deleted marker, or writeRows writing 0 for it, fails these. Trace 2 (rows the JS saved, loaded by the core) and urb-check's worker restart (rows the Rust saved in IndexedDB, read again by a new worker as the same comet) cover the same from the two other sides.

**Every WASI import, supplied.** These are all the WASI imports the two modules ask for, and what `shim/wasi.js` answers. Each answer is what WASI says for the case, with no stub. `instantiate` refuses a module that asks for one wasi.js lacks (`unanswered`), rather than let the browser fail on a missing import or a host stub it.

| Import | Who asks | The answer | Called in the ten traces |
|---|---|---|---|
| `random_get` | the standard library's hash seeds (HashMap's RandomState in the core, serde_json's maps) | open()'s `random(n)`, crypto.getRandomValues by default | once per module |
| `environ_sizes_get`, `environ_get` | libmesa's getenv of MESA_TRACE, through wasi-libc | open()'s `env`, as before | the sizes once; the entries only when env has some |
| `fd_write` | stdout and stderr: libmesa's packet log, a panic's message | lines to `log`, EBADF for any other descriptor | no |
| `fd_close`, `fd_seek` | wasi-libc's stdio | stdio: success and ESPIPE; EBADF for any other | no |
| `fd_prestat_get`, `fd_prestat_dir_name` | wasi-libc's scan for preopened directories | EBADF: there are none | no |
| `proc_exit` | an exit or abort | thrown as an error | no |

mesa.wasm also imports `host.entropy`, the engine's entropy, which the shim takes from open()'s `entropy`. render.wasm asks for the same WASI set and nothing else. Today's module asked for `environ_*`, `fd_close`, `fd_seek`, `fd_write` and `proc_exit`, and seven host hooks; `random_get` and the two `fd_prestat` calls are new. Neither module asks for `clock_time_get` or `args_*`: the core reads no clock, and a cdylib has no arguments. The risk the plan listed ("clock, random, args") comes down to random, which never reaches an output: the replay passes with fresh seeds and with two pinned ones. `random` is a hook of its own beside `entropy`, not entropy's stream: drawing the seeds from entropy would shift the engine's draws, which the traces and rows.mjs pin.

**The order with a store that waits.** The core emits outputs as if the store committed at once, which is how the traces were recorded. A real store answers with a promise, so the shim holds every datagram, the transport's start and close, and anything after an unsettled commit or saveKey, in order, until it settles, as mesa.mjs's `saved` chain did. With a store that answers at once, nothing waits. open() waits for its saveKey and the transport's start, and fails as they fail, so an unreachable relay still fails open() with `can't reach the relay at …`. Nothing queued runs after a failed saveKey or start, so a failed saveKey starts no transport and saves no rows, as when mesa.mjs awaited it before starting. Any failed open() leaves nothing running: the timer is cleared, the core freed, and a transport the core started and didn't close is closed. Today's module leaves the transport started when the saved rows are torn; the shim closes it, and by rule 2 that fix belongs on main too. One real timer is set for the core's earliest deadline and ticks each timer due by name, the earliest first, the first set on a tie. The roller's lookups are given up after 30 s with the JS's DOMException, the bound M3 said the host must keep.

**The harnesses.** Run in /tmp/m6-ext, a clone of nisfeb-extension at e9e5401 (its main), with `shim/build.sh /tmp/m6-ext/lib`. Brave was `/opt/brave.com/brave/brave` (the scripts' default path is not on this machine), and the relay was this branch's Go binary.

- `npm test`: 135 of 135, as with the JS lib/. 78c30bb's suite also passes on the shims, and `node conformance/run.mjs --ext` passes render-ext (334), render-ext-random (3,000) and hostile-render (224) through the shims, hostile bodies within their 2,000 ms.
- `npm run today`: all passed.
- `npm run urb`: run as `npm run urb:fleet` (FLEET=1), all passed (25 checks, the worker restart included, and 30 with the furum part below). The plain script reads `~ricsul-bilwyt`, which this milestone may not touch.
- The furum harness: `FLEET=1 FURUM=~rup/m1c`, all passed: list, post, comment, upvote, and furum's refusal. The script's comment names a snapshot `urb-board` that doesn't exist, and hus-lattice has no board on ~rup, so it ran on m1c-traces, which has board m1c.
- `npm run pub`: all passed, with READER ~hus and the moon's parent ~rup.
- All on the fleet set furum, claimed as lattice-loop-m6 and released after. The last runs were with the final build.

**What the shims can't carry.** Each is loud, not silent, and nothing in the extension uses any of them.

- grub.peek takes a converter by name (`'cord'`, `'text'`, `'exists'`, `'files'`) or as `urb.nouns.cord` and the rest. A converter of the host's own would have to read a noun inside the module, and is refused with a TypeError.
- grub.serve, grub.changed and scry.serve throw: publish() serves, inside the core, through publish.rs.
- grub.onInbox's f is handed `{from, to, mark, jam}`, the noun as its jam, since no noun leaves the module. `nouns` has sham, mug and the converters, not the raw noun readers.
- mesa.mjs's internal exports (the decoders, messages, path, cass, waveOf, snapOf), pub.mjs's markOf and forum.mjs's exact, ids, idOf and actions are not exported. None is in the contract. furum() reads furum at FURUM_BASE only.
- A call the JS threw synchronously (furum's check of a board name) now rejects its promise. An awaiting caller sees the same error.
- uv and da take whole numbers (da whole milliseconds, not negative). The JS took any number.
- Text crosses into render.wasm as UTF-8, so a lone surrogate becomes U+FFFD before the renderer sees it. A body comes through TextDecoder and cannot hold one.
- mdNodes asks hrefFor of every link before it makes any node, in the JS's order and once each, where the JS interleaved the two.
- Anything thrown out of the module stops it: a panic (wasm32-wasip1 aborts, a trap), a proc_exit, a host hook that throws (a log reached through fd_write, entropy, random), a stack overflow. Only the module's own code-1 answer is an ordinary error. A step thrown out of has the core taken out of CORES and may leave the module half way, so the shim rejects every waiting call with `mesa.wasm stopped: …`, refuses later ones with the same, and clears its timer. The reopen from the store that the plan's risks name is not built. It belongs with the soak.
- urb-fetch's own JS CLIs stay on glue.c's module until M7. wasm/pub-cli.mjs wraps grub.serve and scry.serve and imports pub.mjs's markOf, and wasm/check.mjs uses nouns.tree, messages, snapOf, actions and markOf. The shims don't carry any of them (above). The extension and its harnesses use none, but those two CLIs can't run on the shims.

**Not done, and left as it was:**

- No replay of the traces through the shims themselves: trace.mjs's world names each JS timer by its id and records microtask hops the shims don't make. The wasm replay checks the core in the module, and the harnesses drive the shims end to end.
- nisfeb-extension PR #8 (itemText and isPatp) is still a draft. On the shims markdown.js is the Rust, which is linear, so ITEM's quadratic case is gone there, and linkTarget keeps the extension's isShip, as M5b's item 3 left it. PR #10 is untouched.
- lib/ is not swapped anywhere: the gates and the soak come first.

## M7's findings (2026-11-01)

M7's exit criteria passed. Every trace replays through the native host. The Node command lines and the native one give the same exit codes, the same stdout and the same stderr for the same runs, once the masks listed below are applied: all 42 cases of the differential run of 2026-11-01 (after M7's review) are the same, 17 with no ship, 16 read-only, 6 writes and 3 serving. One read-only case, forum-no-post, was run a second time because ~hus gave the Node side no answer at all on the first (the flake below); it was the only retry. The first differential run, before the review, compared no stdout at all (its filter dropped every line without a time stamp), so its "38 of 38" said nothing about stdout; the corrected run is the first that does. `core/ci/cli-diff.mjs` is the differential run, to run again by hand.

**The native host** is `core/urb/src/host`, behind the urb crate's feature `native`:

- `Host` is the driving, the same whatever does the IO. It hands the core each input and acts on each output, in order, through an `Io`, so a commit is saved before the datagrams after it. One timer: it waits for the core's earliest deadline and then ticks the earliest timer due, of two the first set, one a turn. As in the shim, nothing is started or saved after a failed saveKey or transport start, open then fails as they failed, a transport the core started and didn't close is closed, and later calls fail with open's error. A failed commit is logged as `saving the engine's state failed: …` and the engine goes on.
- `NetIo` is the real `Io`. UDP is node.mjs's: one port per identity, kept in the folder's `port` file, galaxies looked up as `<name>.urbit.org` in the background and cached 5 minutes, and on a fake network galaxy g at 127.0.0.1:31337+g. The relay is mesa.mjs's relayTransport over a WebSocket, a small client of its own (ws:// only, as every relay URL so far is), reconnecting every 2 s. HTTPS is ureq with rustls: the roller's lookups are given up after 30 s with AbortSignal.timeout's words, the bound M3 said the host must keep, and a failed request is `fetch failed`, as Node's fetch says. libmesa's own log lines (its notices, and MESA_TRACE's packet log) go to the host's log through `mesa_set_log`, as the Node hosts hand the wasm module's stderr to theirs, so they carry the same time stamp.
- Two stores. `FileStore` is node.mjs's folder: `comet.key` and `rows`, rewritten whole on each commit in a JS Map's order, deleted rows the Map kept included, readRows's leniency on torn files kept. `SqliteStore` is the Go gateway's `<ship>.db`: libmesa's own SQLite store, the same C code the Go binding opens, so a gateway's state opens here as it is, its lock included. One trap: libmesa's `scan` takes an empty prefix only as a pointer that is not null. SQLite binds a null blob as NULL, and no key is `>=` NULL, so a null prefix scans nothing. The engine passes `""`, so only a host is caught by it.
- Comet mining, the roller and the bootstrap, moon keyfiles and `fresh` were already the core's, as fetches and requests (M3). The host gives them a real fetch and a store.

**The command line** is `urb` in `core/urb-cli`: `urb read` is host.mjs, `urb serve` pub-cli.mjs, `urb furum` furum-cli.mjs and `urb forum` forum-cli.mjs, each ported line for line with node.mjs's options. The state folder is node.mjs's (`~/.local/state/urb-fetch-wasm` by default), so a folder moves between the two, which the serve case checks by having each side serve from the folder the other wrote. parseArgs's errors are its own words. Two things the core could not give and now does:

- `Request::Watch`: pub-cli logs each file another ship looks up, each change of the keeps and each scry by wrapping grub.serve and scry.serve, and the core serves inside itself. With Watch on, the core logs the same lines where the wrappers would. No trace sets it. A drop looks its file up too, as pub-cli's wrapper saw.
- What each output means to a command comes through the `Io`'s tap, in order: furum's and forum's notes are logged, and publishing's saves written to pages.json, exactly where the JS's listeners ran.

Fixed after M7's review, each now in the differential run: `--moon` of a directory fails with `EISDIR: illegal operation on a directory, read`, as readFileSync's does (EISDIR names the path except for that read, and a rename names both paths, in Node's words); a failed pages.json save fails `serve` as Node's throw does (put or remove: the error's line and exit 1; while serving: uncaught, exit 1), and what the core logs after the failed save is not printed, since nothing after Node's throw runs. A Udp starts again after a close (a fresh stop flag each start, the old listener joined first), which M8's gateway needs.

The relay's connect and WebSocket upgrade give up after 10 s with `can't reach the relay at <url>`; before, a relay that accepted TCP and never answered hung `open` and every reconnect. mesa.mjs's relayTransport hangs the same way, so by rule 2 the matching fix went to main first as https://github.com/nisfeb/urb-fetch/pull/12 (not merged at the time of writing). Until it lands, the native host is ahead of the JS here, on purpose.

What differs from the Node command lines, by design: an uncaught error (parseArgs's, deUrb's) prints only the error's line, where Node prints its source line and stack too, and a non-UrbError in `read` prints `Error: …` without the stack. The exit codes are the same.

**Every trace through the native host.** `cargo test -p urb --features native --test host` feeds each trace's inputs through the host's `Io`, as a datagram, a fetch's answer or a timer's wake would come, and its calls through the host's calls, and compares what the host does with each output (a datagram sent, rows committed, the transport started or closed, a fetch, a log line, a result handed back) with the trace's, step by step, in order, byte for byte. Each tick must be the timer the host itself would fire at that moment, and the host must have waited for a deadline no later than it. The commits go to both stores in a scratch folder: at the end each holds what the commits leave, and when trace 2 opens again on rows the JS saved, both stores already hold exactly those rows. Trace 5's two early ticks are fired by name, as a clock that fires early would. All ten pass. Dropping small commits fails all ten; ticking the last timer set instead of the first fails eight. Three more tests check a failed start and a failed saveKey (nothing sent, saved or started after, open failing with the store's or the socket's words) and a good open.

**The relay** is tested against the Go relay itself (`tests/relay.rs` builds `urb-fetch` and runs it with `-relay -relay-fake`): datagrams to a galaxy and to an address go out of the relay's UDP port, an answer comes back through the WebSocket with its address, and a relay nobody runs fails the start with the shim's words. The differential run checks the last against Node too. No fleet case runs through the relay: a relayed engine is not a fake ship, so it would look ~hus's keys up on the live roller.

**The differential run.** `FLEET_WHO=<claim> node core/ci/cli-diff.mjs` runs each case with the Node command line (on glue.c's mesa.wasm) and with `urb`, and diffs exit code, stdout and stderr byte for byte once these masks, the same on both sides, are applied (the script lists them too, at `mask` and `normErr`):

- stdout and stderr: our own ship's name where the sides differ (read-only cases run each side as its own fake moon), and the state folder in the cases that print it; lattice's page ids (`0v…`, random per page); in cases that write, the minute of a new post (the sides post minutes apart).
- stderr only: the `mm:ss.sss ` time stamp at the start of a log line and durations such as `1.2s` (the wall clock); and Node's printing of an error's stack, which the native command line leaves out by design: an uncaught error's source block (`file:///…:N` or `node:…:N`, the source line, the caret, a blank line), the `    at …` frames, the `{ code: … }` block after them and the closing `Node.js vN.N.N`. The error's own line is kept.

Nothing else is dropped. A stdout line added to `urb read`'s usage path, and an untimed stderr line there, each fail the diff (checked, then reverted). On the fake network an identity can be used once per snapshot, so read-only cases run on one reset of the furum set, each side as its own fake moon, and a case that writes resets the set before each side, which then runs as the same moon. The cases:

- with no ship (also in CI, `--offline`): usage, parseArgs's errors, a missing, non-moon or directory keyfile, a fake ship with a relay, a fake name that is no ship, a forum url that is no forum, a relay that is down (on trace 1's comet, so nothing is mined), and `serve` on a fake galaxy where pages.json can't be saved (its `.new`, or itself, a directory) or `--put` names a directory;
- read-only: pages of each mark, the home page, the 15 KB page, a missing page, a withheld know entry, `--tries`, `--trace` (stdout and the result line only, since the datagrams are as many as the network made them), furum's list, a thread and a missing board, and a lattice forum's board, a thread and a missing post;
- writes: furum's post, comment and upvote, and a lattice forum's post, comment and upvote, each waiting for its write to appear on the board;
- serving: four pages put (md, an upper-case .HTML, text and one with no extension) and one removed, `--remove` of a page that isn't there, read by ~hus's lattice (`/apps/lattice/fetch`) while it serves, so the watch lines are compared too, then Ctrl-C; each side serving again from the other's folder; and a page name lattice can't take.

**The fleet's own flake.** Some 28 new moons after a reset, ~hus answers no new moon at all for a few seconds, so a read that starts then fails with `~hus answered no lattice peek` after its two 8 s base probes. In cli-diff's read-only phase that lands on the native side each time, since it runs second in each pair; a run of 40 fresh moons, Node and native in turn, lost the 27th (Node's) and the 28th (native) at that same moment, and every later one read. So it is ~hus's, not either engine's. Separately, on m1c-traces ~hus reads `~natdul-molwyl-dozzod-dozhus` a board but never a thread, for both command lines. cli-diff.mjs leaves that moon out. A read-only case is run once more, both sides as new moons and the side that went first now second, only when one side's last line is `~hus answered no lattice peek`; any other difference is reported as it is. Each try keeps its own files, and every retry is printed and counted in the summary. After a reset, ship-cookie is tried up to 5 times while ~hus's web server comes up, each wait printed.

**Not done:**

- Read-only against live ships, as the brief has it, and `--moon` against a real moon (only its error paths are compared).
- The Go gateway's state opened by the native host from a file the gateway wrote: the store is libmesa's own C, so the schema is the same by construction, and the state migration gate (M8) tests it with the gateway.
- wasm/pub-cli.mjs and check.mjs still run on glue.c's module. The native `urb serve` replaces pub-cli; check.mjs stays as the JS's own check.
- No `urb` release or packaging: it builds with `cargo build --release -p urb-cli`.

## M8's findings (2026-11-01)

M8's exit criteria passed, except the read-only run against live ships, which was skipped (below). On the fake fleet the Rust gateway serves the same bytes as the Go gateway for every request in a fixed list of 62: `FLEET_WHO=<claim> node core/ci/gateway-diff.mjs` gave 78 of 78 the same (the 62 requests, 11 startup cases, the kept UDP port, the control and three comet checks), with no retry. `urb_test.go`'s and `reader_test.go`'s cases pass as vectors in Rust: all 128 of conformance/gateway-go.json, and the 60 urls of deurb-go.json through the gateway's deUrb. `core/ci/test.sh` with REQUIRE_ALL=1 passes, and `go test ./...` too.

**The gateway** is `core/urb-gateway`, a binary `urb-gateway` with the Go gateway's flags (Go's `flag` package's parsing, errors and usage, in `flags`), and a library the tests use:

- `routes` is main.go's `ServeHTTP`, `serve`, `render`, `redirect`, `home` and `error`, byte for byte, with urb.go's deUrb, urbOfHost and gatewayURL. The reading is handed in as a function, so the routes run without a network in the tests.
- `http` is the part of Go's `net/http` the answers show (and, since the fix round below, its server's handling of slow and malformed requests too): the host from the absolute URL or the Host header, `net.SplitHostPort`, the path percent-decoded as `url.ParseRequestURI` decodes it (a bad escape or a control byte is a 400 before any handler), the query as `url.ParseQuery` takes it (a pair with `;` or a bad escape dropped, `+` a space), `strings.TrimSpace`, `http.Error`, `http.Redirect` (its `hexEscapeNonASCII` writes lower-case hex), the status line in the request's HTTP/1.x, and Go's words for the requests its server refuses (a missing, malformed or doubled Host header, a bad request line or escape, HTTP/2 on HTTP/1). Paths and queries are bytes, since a decoded one need not be UTF-8. The server is a small HTTP/1.1 one on threads, with Go's timeouts (see the fix round).
- `state` opens the state folder as main.go's `open` does: `comet.key` (a new comet is mined by the core and saved as `mesa.LoadComet` saves it, created and never overwritten, 0600), `<name>.db` (`fake-<name>` for a fake ship) through libmesa's SQLite store, and `<name>.db.port`, with the binding's words for each failure (`mesa: not a ship: "~x"`, `<path>: mesa: not a keyfile`, `mesa: <db> is in use by another process`, os.PathError's `open <path>: …`, the moon with no state).
- `golog` is Go's `log`: the local `2006/01/02 15:04:05 ` before each line. The gateway's log lines are the Go's too: on the fleet run the two logs are the same once the time and the durations are masked.

The host runs on a thread of its own. Each HTTP thread hands it a read and wakes it with `Event::Wake` (`NetIo::waker`), new in the host, and `Udp::with_port` takes the port the folder keeps. `-relay` is refused with exit 2: the relay stays in the Go binary, as the plan has it.

**The reader.** The Go gateway reads with its own reader.go, which differed from mesa.mjs in five ways its answers show. M8 first kept them behind a switch in the core, `Request::Gateway(true)`. The review showed each was a bug in mesa.mjs, so by rule 2 they were fixed there and ported, and the switch is gone (see the fix round): the core has one reader, mesa.mjs's, for the extension and the gateway alike. What stays the gateway's own is presentation, in urb-gateway: `routes::fail_of` gives each of the core's failures reader.go's status and words.

**The vectors.** `gateway_vectors_test.go` writes conformance/gateway-go.json from the Go with UPDATE_VECTORS=1 and holds the Go to it: each test's own inputs from urb_test.go (deUrb, gatewayURL, urbOfHost, looksHtml, rewriteHTML, the three renderers whole, renderAs, the page marks, the gateway's render with its CSP, safeLink, the error page's status, headers and body, oneIdentity) and from reader_test.go (isPage, and TestLateAnswer as `lateAnswer` and `onTimeAnswer`). `cargo test -p urb-gateway --test vectors` runs 126 of them, and deurb-go.json; `cargo test -p urb --test gateway` runs the two reader cases as scenarios on a core with a fake engine, ~zod's answers handed to its hooks: the late list kept (`{kept, waiting}`), and an answer in time going to the read with nothing late (`{caller, late}`), on the core's one reader. Each file's count is checked.

**The differential run.** `core/ci/gateway-diff.mjs` is also the state migration test. The Go gateway runs as the fake moon ~natdul-molbud-dozzod-dozhus on a fresh folder and answers the list; it is stopped (Ctrl-C), its folder is copied (`.db`, `-wal`, `-shm`, `.port`, `.lock`), and the Rust gateway opens the copy as the same moon and answers the list again. On the fake network an identity heard before can't start again from empty: the control case, the same moon on an empty folder, gets no answer (504). So every page the Rust side reads shows it carried on the Go gateway's flows from its `.db`, with no breach and no stall. While it runs, the Rust gateway must have bound the UDP port in `<name>.db.port`. The comet case runs offline, in a network namespace with only loopback (`unshare -rn`): `core/ci/cometkey` mines a comet under ~marzod without the live comet-stars list and writes `comet.key` as LoadComet does, the Go gateway opens it (writing `<comet>.db` and its port), and the Rust gateway opens a copy: the same home page, naming the comet, the same port bound, and no file of its own.

The list: the home page by each of its hosts and by HEAD, /go to a page, with spaces and non-ASCII and HTML in the url, by HEAD, to no urb:// url, to no ship, with none, with `;`, a bad escape and two values; POST and PUT; an unknown host, a subdomain, a name that is no ship and one in upper case; ~hus's index, fleet-check by GET, `?raw`, `?raw=0`, HEAD, a trailing slash, /n/ and no port; the 15 KB page (`big`, which traces/fixtures.sh puts in the m1c-traces snapshot) and its raw; pages of each mark (md, html, text, gmi, css); an encoded slash; a listed id nothing answers; a missing page and a missing index; /k/, /t/, /p/, a single-letter mount, a bad path, a space, an empty segment, /n alone; a forum board, a post and /forum; ~rup, which runs no lattice; ~fes, which runs nothing (30 s each side); and raw requests: no Host, HTTP/1.0 with and without one, a bad Host, two Hosts, a bad escape in the path, an absolute URL, HTTP/2.0 and a request line that is no request.

The normalisation, the only one: the Date header (the wall clock), and Content-Length, Transfer-Encoding and Connection (framing: Go chunks a body longer than its 2 KB buffer and says Connection: close as it closes, where the Rust gateway always sends a length; each body is compared de-chunked, byte for byte, which fixes its length). The status line, every other header with its name, value and order, and the body are compared as they came, and the requests Go refuses before its handler are compared whole. The startup cases mask only the log's time and the program's path in `Usage of`. A mutation that adds one newline to each rendered page fails the run (fleet-check and marks-md differ, exit 1); it was reverted and the binary rebuilt before the counted run. The first run differed in three raw cases, each a Rust bug, fixed: the status line answers in the request's HTTP/1.x, Connection is said as Go says it, and two Host headers are Go's bare 400.

**Not done, and differences left:**

- **Live ships: skipped.** Reading a lattice page needs its revision list, and the only way to read it is grubbery's cross-ship peek, which is a `%grubbery-load` poke to the publisher (the vault fallback is one too); only the page itself is a plain remote scry. Every new identity also leaves a permanent record on the ship it reads. So a read of a live ship's public page is not cleanly possible with no pokes and no writes, and none was made: no live ship was contacted at all.
- A page body that is not UTF-8: the Go gateway passes its bytes on, and the core hands the body over as a string, as mesa.mjs's TextDecoder does, so such a body would differ. Lattice publishes cords, and none on the fleet is such.
- Framing: always Content-Length, never chunked, and Connection, Date and Content-Length are written after the handler's headers in that order. A chunked request body closes the connection after the answer, and `Expect: 100-continue` is not answered with a 100.
- A moon on the real network (`-moon` that has state, or `-new-moon`) was not run; only its refusal with no state is compared.

### M8's fix round (2026-11-01)

The review (FIX FIRST) found the server looser than Go's against slow clients, and the reader switch against the plan's first goal. Both are fixed. Two fixes went to main first (rule 2), neither merged:

- https://github.com/nisfeb/urb-fetch/pull/13 (JS): mesa.mjs reads as reader.go where reader.go was right. A base probe nobody answers ends the read at 30 s (no second probe), still `later`; a ship that answers both probes with nothing is no longer `later` (same words), which is how the gateway tells its 504 from its 502. A list refresh nobody answers keeps the stale list (an empty one read every page by the vault peek, so markdown showed as gemtext for 60 s). One read looks each page up in the list it got. A list that answers after its read gave up is kept, with its base, for 5 minutes (`LATE_WAIT`, logged as `a late answer came in on <wire>`). An entry with no id is no entry. `wasm/test/reader.test.mjs` has a case for each, on mocked timers; each fails on main.
- https://github.com/nisfeb/urb-fetch/pull/14 (Go): the gateway's server had only ReadHeaderTimeout, so a kept-alive connection waiting for its next request, and a request whose announced body never came, were held forever. `newServer` adds ReadTimeout and IdleTimeout, 10 s each. `server_test.go` checks each.

Both are cherry-picked onto rust-core. Then:

- **The reader.** The core ports the fixed mesa.mjs, and `Request::Gateway` is removed: nothing is left that only the gateway should get. Traces 6, 9 and 10 are recorded again with the fixed JS (the same results in fewer steps: trace 6's offline ship fails after one probe). `core/urb/tests/gateway.rs` runs reader_test.go's two vectors and each behaviour on the one reader, with the missing case for one list per read.
- **The server.** Every read has Go's deadline, kept by poll(2) to the millisecond (SO_RCVTIMEO ran 2 to 3 % late): 10 s for a head from the connection's start or the next request's first four bytes, 10 s for its body from the request's start, 10 s idle between requests. At most 512 connections are served at once, over both listeners (a browser opens at most 256 sockets in all); more wait in the listen backlog, which is Go's (net.core.somaxconn), not std's 128. Past std's 128 a client's connection looked open and was never accepted.
- **Request parsing** ports net/http's order: textproto's header reading (an obs-fold joins its line, a space in a name reaches the server's `invalid header name`, a control byte in a value is a bare 400, 500 lines at most), the version (`HTTP/1.10` is a bare 400, `HTTP/0.9` a 505), Transfer-Encoding (only one `chunked`, from HTTP/1.1, else Go's 501), Content-Length (no sign, so `+0` is a bare 400, and doubles must agree), a trailer that names a framing header, `Expect` (anything but 100-continue is a 417), `OPTIONS *` (Go's own empty 200) and CONNECT's authority. An unread body is handled as Go handles it, after the handler and before the answer: discarded if under 256 KB and in time, chunked too, else the connection closes after the answer.
- **Startup words.** The gateway mines a comet itself, as the binding's LoadComet does, with its words for each failure. A UDP port in use is `listen udp4 <addr>: bind: address already in use`.
- **The host's thread.** If it ends on its own the gateway exits (2 for a panic, Go's code for one), rather than answering 502 forever.

**The differential, again.** `core/ci/gateway-net.mjs` (run by gateway-diff.mjs, with `--offline` too, so in CI) puts both servers side by side as the fake ~zod in a network namespace with only loopback: 36 raw requests (the review's probes, and those above), 6 slow clients (idle, kept alive, a trickled head, a trickled body, an unsent body with and without Connection: close), which must say the same and close within 0.5 s of each other, a flood of 300 idle and 30 trickling connections during which a request must answer within a second and after which all must be closed by 12 s, and two startup failures (the kept UDP port in use, a comet mined with no network). Offline: 54 of 54 the same. On the fleet (`FLEET_WHO=lattice-loop-m8fix`): 123 of 123, no retry. The urb-gateway unit tests also hold 180 slow clients on short timeouts while a request answers, and check the cap.

**What's left differing:**

- Over 512 connections at once, a request waits for a slot (at most about 10 s, as the timeouts free them), where Go accepts every one. No browser page can reach it.
- A comet mined with no network: after `Get "<url>": ` the transport's own words differ (Go's resolver against ureq's).
- Framing, non-UTF-8 bodies, `-relay`, and the real-network moon, as above.

## Testing beyond the traces

- **The fake fleet, automated.** One script resets the furum set to `hus-lattice` and runs the stage 3, furum and lattice forum checks against the JS engine and then the Rust engine, with the same assertions. Run it for every milestone from M3 on.
- **State migration.** Take state written by the JS engine (a comet and a fake moon that have talked to `~hus` and `~rup`) and by the Go gateway. Open it with Rust. It must continue with those ships with no breach, no stall, and pages and keepers carried. A fake ship that remembers the identity's sequence numbers is what proves nothing was replayed.
- **Memory safety at the C boundary.** Native test runs under AddressSanitizer and LeakSanitizer, libmesa included.
- **Fuzzing.** `cargo fuzz` on every decoder of another ship's data, on `deUrb`, and on every renderer. Each fuzzer runs for hours before M8 exits, and in CI for minutes on every change.
- **Timing and size.** Record today's numbers first: reads 0.3 to 2 s on the fleet, an in-process page read 16 ms, a cold first read in headless Brave 4 s. The Rust builds must be no slower, and wasm within its size budget.
- **Live network, read only.** Reads from `~ricsul-bilwyt`'s public pages, from one persistent comet and not a new one per run, a few times per gate. Routine and CI runs use the fleet only: every new comet leaves a permanent record on the ship it reads (17 so far). Never a write to a real ship. Never the owner's moon `~mister-botter-dozzod-nisfeb`. A real test moon only once its key is in `~/.config/urb-browser/test-moon.key`.

## Release gates

Each consumer moves only when every gate passes, in order.

1. **Parity.** Every vector, every trace, the composed-shape clams, the fuzzers, and the sanitizer runs pass.
2. **The fleet.** The automated fleet suite passes for JS and Rust with identical results, three runs in a row.
3. **Migration.** Every saved-state case continues without breach or stall.
4. **The consumers' own suites.** The extension's harnesses, and a gateway side-by-side diff, as in M6 to M8.
5. **A soak.** The extension built on Rust runs in a Brave test profile, and the Rust gateway on this machine, for a week of normal use. Any difference from the JS engine stops the release until it is explained and fixed.
6. **Release.** The extension's `main` takes the new `lib/` files in one commit, so that reverting it restores the JS engine. The gateway's Go binary stays buildable for one release after the switch.

## Later consumers

**Lattice desktop (Tauri).** It does not use urb-fetch's engine today. When it does, it takes the `urb` crate with the `native` feature, and `urb-render`, as Cargo dependencies pinned to a urb-fetch commit, and runs the native host in its own process. It does not use the wasm build, so WebKitGTK's wasm does not matter to it. It has no SQLite of its own today, so it takes libmesa's. If it gains one, the native build links libmesa's store against that copy instead (see "SQLite twice"). The desktop is not a gate for any release in this plan. It gets its own plan once the port has shipped.

## Rollback

- The extension: revert the one commit that swapped `lib/`. Saved state is the same format both ways, so a rollback loses nothing.
- The gateway: run the Go binary again. Its state folder is untouched in format.
- Anything that writes state in a new format is out of scope for this plan, so a rollback never meets state it can't read.

## Risks

- **Building libmesa with `cc` instead of zig** could change compiler flags, defines or optimisation. Mitigation: replicate `build.zig`'s explicit flags exactly, run libmesa's own suites against our build and against zig's at the same baseline CPU, record both C compilers in each run, and replay the traces.
- **SQLite twice.** libmesa bundles an unprefixed SQLite; the desktop app and Ladybird may link their own. A native build inside one of them links libmesa's store against the host's SQLite instead of bundling a second copy. The command line, which has no other SQLite, keeps libmesa's.
- **Rust's standard library on WASI** asks for imports the current shim doesn't give (clock, random, args). The shim must supply each, or the build must avoid them.
- **Panics.** A panic in wasm stops the module. The core must not panic on any input; tests and fuzzers check it. The wasm host reopens the engine from its store if it ever happens.
- **Upstream changes during the port.** Lattice, grubbery and furum keep releasing. The traces and vectors pin today's shapes. When an upstream change lands, the JS is updated on main first, the traces are re-recorded by the CI job, and then the Rust follows.
- **Two renderers as references.** The Go and the JS renderers differ today. Until each difference is settled on main, one Rust renderer cannot match both. M1 settles them first.
- **grubbery b126c50 and eed73a6.** `~ricsul-bilwyt` runs b126c50, by fingerprint, and the fleet runs eed73a6. Serving to a b126c50 reader is untested today for the JS too. Add a fleet ship on b126c50 before gate 2, or record the gap.
- **The ricsul pin is a fingerprint, not a marker.** The ship states no commit. If a later fingerprint matches nothing, the shapes are checked against the ship's files directly before any trace that depends on them.

## Open decisions

1. **Whether the extension moves to the Rust wasm build at all.** Decide at gate 4, on size and on the soak.
2. **When the Go gateway is retired.** This plan keeps it buildable for one release after the switch.

Where it lives was decided 2026-10-30: urb-fetch, beside libmesa, the wasm build, the gateway and the fleet tests.

## Effort

5 to 6 weeks of work for M0 to M8, plus the week's soak. The milestones above add up to about 27 days, with M1 at about a week, since the recorder, nine traces and byte-for-byte replay are the hardest part. Fixing what the traces find is the unknown, and the estimate allows for some of it.
