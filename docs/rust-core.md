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

**The server** is mesa.mjs's, in `core.rs`. A `%peek` of a file we serve is answered with its `%snap` and silo inline, a `%keep` with its first wave and, if the file holds something, kept (up to 256 keeps, `MAX_KEEPERS`; a keep held already and unchanged is not saved again), a `%drop` forgets the keep, and `grub.changed` sends each keeper of a file its new wave, in one transaction. A `%keen` of our namespace is answered by `scry.serve`. `grub.serve`'s three functions are closures (`Files`, `Scries`, `OnKeepers`) the engine's state holds, so the shims of M6 can hand in the JS's own. `publish.rs` is pub.mjs: `pageKey`, the index, revs and vault files, the page scry, `put` and `remove`. The calls are trace.mjs's `pub`, `pub.put`, `pub.remove` and `pub.pages`, and each save is an event, `pub.save`.

**meta.pub is kept as JS values, not as a struct.** `publish.rs` holds the saved `{pages, keepers}` as `js::Json`, in the order the JS keeps an object's keys, so whatever a host saved is saved again as it was: a page from before marks keeps having none, a removed page has none either, and a field the JS never wrote survives. `js::stringify` is JSON.stringify. `tests/publish.rs` publishes from 16 saved states (trace 5's saves, an empty one, and older and odder shapes: keys pageKey never makes, revs and times that are fractions, text or missing) in pub.mjs on mesa.wasm (`tests/pub-oracle.mjs`) and in the core, runs 43 ops on each (puts and removes with Unicode bodies and names, and other ships' peeks, keeps and drops), and requires the same results, the same 273 saves as JSON text, the same pokes by sham and the same log lines, byte for byte.

**Port the JS's accidents, again.** put's rev is `(rev ?? 0) + 1` and remove's is `rev + 1`, as JS adds (`js::add`: a saved rev of `"2"` becomes `"21"`), so a saved page with no rev comes back up at 1 and goes down at NaN. A rev or time that is not a whole number fails where the JS's `BigInt()` and `da()` fail, with their RangeError, TypeError or SyntaxError word for word (`js::bigint`, `msg::da_of`), so `msg::File` keeps the rev and time as the JS values they are. The index reads each page's hash as `BigInt('0x' + hash)`, which throws for a page saved with none: a peek of the index logs the error and is not answered, and every put fails after its save. The index's key is the page's key less its first UTF-16 unit, as `k.slice(1)`, so a saved key that is empty or starts with a character past the BMP lists, and doesn't panic, as the first port did (the review's must-fix). A saved keeper whose ship is not one makes every put of its page fail with `not a ship: …`, after the save. Each needs a state pub.mjs never writes.

**Two fixes went to main first** (rule 2, PR #8), from the review: one ship could hold all 256 keeps with keeps of pages we never published, saved across restarts, and every repeated keep saved the whole of meta.pub again. Now only a file that holds something is kept, and an unchanged keep is not saved. Both are ported. One ship can still hold a keep of every page we do publish; a cap per ship would cut off a reader that follows a big site.

**The molds.** `capture_pokes` keeps the mark and jam of every poke the core makes on its thread. The replay test clams each distinct one, from every required trace and from a serving session of peeks, keeps, edits and a drop from another ship, against `load`, `transfer` and `intake` in check.mjs's copy of grubbery's molds, by `(sham ;;(mold (cue …)))` against the Rust sham. 49 distinct messages fit: 20 load `%peek`, 1 load `%poke`, 17 transfer `%snap`, 1 `%pack`, 1 `%want` and 9 intake waves. A load with a deep flag of 2 and a `%snap` whose pace is `%fir` must be refused, and the same `%snap` with `%firm` must fit.

**The replay bites** for serving too. Each of these fails trace 5: no room for a keep, the page scry's id cut one short, the page hash in upper case, the list's version one higher, and a keep that does not save.

**Fuzzing.** A new target, `serve`: a core publishing pages, one taken down and one kept by a saved keep, hears a `%grubbery-load` whose whole noun, wire, dest or folder is the fuzzer's, for each of `%peek`, `%keep` and `%drop`, or a scry of the fuzzer's bytes as a path; then an edit waves every keeper and the timers run. Its pick 5 publishes from a saved page whose key is the fuzzer's text, seeded with the empty and multibyte keys, with a whole or fractional rev, and peeks the index and the list. No hook may panic, every noun must be let go, the keeps must stay under the cap, and what it gives must stay under `OUT_MAX`. `seed.mjs` seeds it. A brief run found nothing: serve ran 41,877 inputs in 12 minutes with AddressSanitizer and LeakSanitizer and 60,931 in 5 without, and hooks 21,747 in 3 without. After the review's fixes, with pick 5, serve ran 12,451 in 6 minutes with them and 23,904 in 3 without, and found nothing. CI runs it for a minute with the rest.

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
