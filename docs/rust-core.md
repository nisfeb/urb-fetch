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
- **Grubbery b126c50 on `~ricsul-bilwyt`.** The ship has no version or commit marker: its docket says version 1.1.0, which is lattice's docket, not a commit. So the commit was found by fingerprint, read-only, from the ball's mirror of the clay desk (`/grubbery/api/file/sys/clay/desks/grubbery/...`). Of the 107 desk files outside `gub/`, 104 match b126c50 exactly, including `app/grubbery.hoon`, `lib/nexus.hoon` and `lib/tarball.hoon`. The other three are the docket and two test files. Of the files that changed between eed73a6 and b126c50, `gub/mar/clay/trunk/trunk-action.hoon` and `gub/nex/shell.hoon` match b126c50 and not eed73a6. No later commit matches. **Open for M1:** fingerprint the whole desk, `gub/` included, before any trace is recorded, and again whenever the ship takes an update.
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

- **`mesa-sys`** builds libmesa from `third_party/libmesa` with the `cc` crate, with the same flags as `build.zig` (`-std=gnu23`, `-DU3_OS_ENDIAN_little=1`, `-DU3_OS_PROF=1`, the OS define, the urcrypt subset) and the wasm diff applied for wasm builds. Its bindings are generated by `bindgen` from `include/mesa/*.h`, never declared by hand: a signature that drifts from the header is silent undefined behaviour. It holds no logic.
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

**Recorded sessions.** libmesa's engine is deterministic: it reads no clock and no OS randomness (its source has none outside its test helpers; confirmed in M0). So with the clock and entropy fixed, one session's inputs fully decide its outputs. A recorder wraps the JS host and writes every input and every output to a trace file:

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
- libmesa's own test suites run against our build, natively and under WASI (11 of 12 pass under WASI today; store_test needs flock).
- Confirm the engine reads no clock or randomness of its own.
- CI runs `cargo test`, the conformance suite and `check.mjs`.
- Exit: libmesa's suites pass with the same results as its zig build.

**M1. The test bed, in JS** (about a week)
- Fingerprint `~ricsul-bilwyt`'s whole grubbery desk, read-only, and confirm b126c50 or update the pin.
- Injectable clock and entropy in `mesa.mjs`, and the hardened decoders, released on main on their own.
- The lattice forum reader in JS, released on main on its own.
- The Go renderers given 800ff22's rules, and the other renderer differences settled, on main.
- `npm run urb` given a fleet target (`hus-lattice` serves lattice, updated to release 81), so routine runs stop mining a comet against `~ricsul-bilwyt` each time.
- The recorder, and traces 1 to 9.
- The conformance vectors, generated, passing in JS and Go.
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
- Exit: trace 5 replays; every built message clams against grubbery's molds in vere; trace 7 replays.

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

- **Building libmesa with `cc` instead of zig** could change compiler flags, defines or optimisation. Mitigation: replicate `build.zig`'s flags exactly, run libmesa's own suites against our build, and replay the traces.
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

5 to 6 weeks of work for M0 to M8, plus the week's soak. The milestones above add up to about 26 days, with M1 at about a week, since the recorder, nine traces and byte-for-byte replay are the hardest part. Fixing what the traces find is the unknown, and the estimate allows for some of it.
