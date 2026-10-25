# hltv-demo — specification

Find and download professional CS2 demo archives from hltv.org through a real browser.

```
                    ┌── CLI (src/cli.rs, bin hltv-demo)
search  filters ──► │                        ──► matches          (what exists)
resolve match   ──► │  shared operations       ──► demo archive URL (does this match have a demo)
download URL    ──► │  src/hltv/*              ──► .rar on disk     (get it)
                    └── MCP server (src/mcp.rs, bin hltv-demo-mcp, stdio)
```

Status: v0.2, 2026-10-25. This Rust crate replaced the TypeScript implementation (v0.1) that day.
Everything under
[Verified facts](#verified-facts) was measured against live hltv.org; everything else is design. What
is still unproven is listed under [Open questions](#open-questions) — it is not implied by the checks
below.

## Purpose

Turn an hltv.org URL, or a filter over recent results, into demo archives on disk. It is a library
plus a CLI, with no knowledge of any downstream consumer: archives keep HLTV's own file names.

## Non-goals

- **No Cloudflare bypass.** No headless trick, no TLS impersonation, no proxy. A real, headed
  Chrome answers the challenge, and a human answers it once per profile when it appears. Shipping a bypass
  is out of scope permanently, not deferred.
- **No crawler.** No "download every demo of event X", no unbounded paging, no background daemon.
  Every list has a limit and there is a delay between requests.
- **No demo parsing, storage service, or database.** The output is files.
- **No downstream integration.** A consumer reads the directory; this tool does not import one.

## Verified facts

Measured on macOS with Google Chrome 155: rows dated 2026-10-24 from the first Rust implementation
and the TypeScript one, rows dated 2026-10-25 from this one.

| Fact | Evidence |
|---|---|
| Every non-browser request is challenged: `403`, `cf-mitigated: challenge` | `curl` on `/results`, `/matches/<id>`, `/download/demo/<id>`, even `/rss/news` |
| Headless Chromium is challenged even with a warm profile | `--headless` run against the same profile that passes headed; 2026-10-25: a fresh headless profile got "Just a moment..." |
| A headed Chrome with a fresh persistent profile passes in ~1.2 s | Playwright, `channel: "chrome"`, `headless: false` |
| `/download/demo/<id>` refuses everything but a browser navigation | `APIRequestContext` with the browser's own cookies and UA, after a successful page load: `403` |
| A match page exposes its demo as `a[href^="/download/demo/"]` | one per match, including matches that later lose it |
| One archive holds every map of the series | Vitality vs MOUZ bo5 → 3 `.dem` (cache, inferno, dust2); bo3 → 2 |
| Archive names are HLTV's own: `<event>-<team>-vs-<team>-bo<n>-<id>.rar` | e.g. `pgl-masters-bucharest-2026-vitality-vs-9z-bo3-eP_cF4LqZBl7PERj7oPEob.rar` |
| Archive sizes are 440–890 MB | measured over 5 archives |
| Download time is the network's, not the tool's | 2026-10-25: the same 480 MB archive back to back, Rust 23.9 s then TypeScript 38.5 s, byte-identical; of a 703 MB download's 27.8 s, launch, arrival, start, and close took 1.6 s |
| Chrome **overwrites** an existing download of the same name (no " (1)") | a repeat navigation to the same demo id rewrote a 703 MB file |
| `Page.navigate`'s reply can arrive long after the page loads, and never arrives for a download | 25–30 s per page in the first implementation |
| A download in progress is `<guid>.crdownload` under `allowAndName`, renamed to `<guid>` when complete | 2026-10-25: a SIGINT at 7 MB showed `4a1ac1f1-….crdownload`; nothing was left after |
| A navigation that fails at the network commits `chrome-error://chromewebdata/`, titled with the host | 2026-10-25: `Offline`, where no host resolves |
| Cloudflare's challenge rewrites the URL it is solving with `__cf_chl_rt_tk=…`, then restores it | 2026-10-25: the challenge probed every 250 ms |
| chromiumoxide adds the `--` to a switch: passing `"--x"` launches Chrome with `----x`, which it ignores | 2026-10-25: a stand-in Chrome recording its argv |
| A results page shows a match twice: in the "featured" block and again under its date heading | the featured copy has no date heading, so a naive `--limit 1` returns a duplicate |
| `?offset=N` pages the results list | `--offset 1` on Vitality/Cache returned [2398757, 2398754] against [2399723, 2398757] at offset 0 |
| Two runs on one Chrome profile corrupt each other | a backgrounded results run and a foreground demo run shared the profile and downloads crossed over |

## Two entry points, one core

The CLI and the MCP server are transports over the same three operations. Neither contains a rule:

| Layer | Owns |
|---|---|
| `src/core/*` | URL grammar, page identity, dates, manifest, profile lock, the error table |
| `src/browser.rs` | chromiumoxide: headed Chrome, launch switches, page reads, download completion |
| `src/browser/arrival.rs` | what one observation of a page means: wait, read, or fail (no browser) |
| `src/source.rs` | the browser boundary (`Source`, `Opener`) and `with_source`, one browser per unit of work |
| `src/hltv/*` | `search`, `resolve`, `get` over a `Source`, and the in-page readers |
| `src/cli.rs`, `src/mcp.rs` | arguments, formatting, exit codes / tool results |
| `src/process.rs` | what both binaries share and the library does not install: stderr logging, signals |

Both validate a URL (and a map name) at their own boundary, before a browser opens, so a bad input
is instant in either.

## CLI contract

```
hltv-demo search [--team ID] [--map NAME] [--limit N] [--offset N] [--json]
hltv-demo resolve <match-url> [--json]
hltv-demo download <url> [--out DIR] [--limit N] [--delay MS] [--json]
```

MCP tools (stdio, `hltv-demo-mcp`): `search_matches {team?, map?, limit?, offset?}`,
`resolve_demo {matchUrl}`, `download_demo {url, out?, limit?, delayMs?}`,
`list_downloads {out?}`. Each publishes an input and an output JSON Schema. Numbers are integers; a
negative `limit`, `offset`, or `delayMs` reads as 0, and the operation's own rule applies from there
(a limit is clamped to 1..100). Tool failures carry `{ message, remediation }`; a failure is a tool
error result, never a protocol crash. Results come as structured content and as the same JSON in a
text block, for clients that read only text.

- `<url>` is any hltv.org match, results, or `/download/demo/<id>` URL. Bare paths are accepted.
- `--limit` defaults to 1 and caps how many matches a results URL contributes.
- `--out` defaults to `~/.cache/hltv-demo/archives`.
- `--json` prints one JSON object per line and nothing else on stdout; human output goes to stderr.
- Exit codes: `0` success, `1` failure, `2` usage or missing prerequisite (no Chrome, a held
  profile, no network), `128 + n` when signal `n` stopped the run (130 for Ctrl-C), after Chrome is
  closed and the profile released.
- `hltv-demo` with no command prints the usage and exits `0`.
- `RUST_LOG=hltv_demo=debug` prints launch, arrival, and download timings on stderr.

Examples:

```bash
hltv-demo search --team 9565 --map de_cache --limit 5 --json
hltv-demo resolve https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026
hltv-demo download 'https://www.hltv.org/results?team=9565&map=de_cache' --limit 2
hltv-demo download /download/demo/113159 --out ./demos
```

## Data model

The JSON both transports emit, in this field order. Each shape is one struct in `src/core/types.rs`,
and the MCP output schemas are derived from the same structs.

```
Match   { matchId: number, url: string, date: string | null, event: string | null,
          teams: [string, string], scores: [number, number] | null }
DemoRef { matchId: number, demoId: number, url: string, matchUrl: string }
Archive { path: string, bytes: number, demoUrl: string, suggestedName: string }
```

`url` is absolute; `date` is an ISO day from the results-list heading.

## Invariants

1. **hltv.org only.** Scheme and host are checked before a browser opens; anything else is
   `NotHltvUrl`. Bare paths are resolved against `https://www.hltv.org`.
2. **No archive without a completed download.** Chrome writes under its download guid
   (`<guid>.crdownload`, then `<guid>`); the file gets HLTV's name and a manifest line only after the
   browser reports the download complete. A run that ends any other way — a stall, a cancel, a
   signal — removes the partial file.
3. **A demo URL is downloaded once** while its file exists. `<out>/.hltv-demo.tsv` maps demo URL →
   file name; a missing file invalidates the entry.
4. **Page identity, not load state.** "Arrived" means the URL matches by match id / demo id (slug and
   query ignored), or for any other page by path and query (fragment and Cloudflare's `__cf_chl_*`
   parameters ignored). `readyState` from the previous page is never trusted.
5. **One browser, one process.** A profile lock (pid file) makes a second concurrent run fail with
   `ProfileBusy` instead of sharing the browser and mixing its downloads. No parallel requests.
6. **A results URL keeps its own filter.** `/results?team=9565&map=de_inferno` must never widen to
   `/results`; the query is parsed, not discarded.
7. **A match appears once.** Featured and dated copies of the same match id collapse, keeping the copy
   that carries a date.
8. **One core, two transports.** The CLI and the MCP server call the same operations; a rule added for
   one is a rule for both, and neither may grow its own URL or skip policy.
9. **Input is validated where it enters.** Both entry points reject a non-hltv.org URL and a map name
   that cannot be one before opening a browser, so a typo never costs a Chrome launch.
10. **A stopped run cleans up.** SIGINT, SIGTERM, or SIGHUP closes Chrome and releases the profile
    lock before either binary exits; a client leaving the MCP server does the same for a call still
    running.
11. **A challenge gets a bounded wait, never a bypass.** A challenge that stands for 30 s is reported
    as `Challenge`; one that clears — Cloudflare's own check in a headed browser, or a person
    answering it in the window — lets the read continue.

## Errors

Typed, exhaustive (`HltvError` in `src/core/errors.rs`), each with a remediation a user can act on:

| Error | Meaning | Remediation printed | CLI exit |
|---|---|---|---|
| `NotHltvUrl` | not hltv.org | show the accepted shapes | 1 |
| `UnsupportedPage` | hltv.org, but not match/results/demo | point at the three shapes | 1 |
| `NotAMap` | a map filter that cannot be a map name | an example map | 2 as `--map`; 1 inside a results URL |
| `Challenge` | a Cloudflare challenge stood for 30 s | the exact `open -a` command and profile path | 1 |
| `NoDemoFound` | match page has no demo link | "the match has no demo (or HLTV changed its markup)" | 1 |
| `NoMatchesFound` | results page listed nothing | the filters and the URL | 1 |
| `DownloadFailed` | canceled, stalled 60 s, never started, or not saved | retry; nothing partial is recorded | 1 |
| `BrowserMissing` | no Chrome at the path, or it would not launch | `CHROME=/path/to/chrome` | 2 |
| `BrowserFailed` | a tab would not open, the page threw, or it opened another page | close the window and retry | 1 |
| `ProfileBusy` | another run, or another Chrome window, holds the profile | wait, close that window, or `HLTV_DEMO_PROFILE=` | 2 |
| `Unreachable` | hltv.org did not answer: no network, no DNS, refused | check the network | 2 |
| `ManifestUnreadable` | `.hltv-demo.tsv` is corrupt | delete it; archives on disk are kept | 1 |

## Search capability matrix

`search` reads `/results` and nothing else. Honesty about what is proven:

| Filter / behavior | Status |
|---|---|
| `?team=<id>&map=<de_map>` | **verified** — the URL shape the nades `just coverage` links use |
| `?offset=<n>` paging | **verified** — offset 1 skipped one result |
| `?event=<id>` | unverified |
| date-bounded listing (`/stats/matches?startDate=&endDate=`) | out of scope; it lists per-map stats pages, a different identity model |
| a match's demo availability | only from the match page: `search` never claims a demo exists |

## Architecture

```
src/core/              url.rs (parse, classify, page_key) · dates.rs · manifest.rs · lock.rs · errors.rs · types.rs
src/browser.rs         Chrome over CDP: Session (headed, shared profile, locked), Offline (headless, no network)
src/browser/arrival.rs what one probe of a page means: wait, read, or fail
src/source.rs          Source and Opener, and with_source (one browser per unit of work)
src/hltv/              extract.rs + extract.js (in-page readers) · search.rs · resolve.rs · get.rs
src/cli.rs             clap arguments, human output, JSON lines, exit codes
src/mcp.rs             rmcp tool router: the same operations as four MCP tools over stdio
src/process.rs         stderr logging and shutdown signals, for the two binaries only
src/main.rs            the hltv-demo binary · src/bin/hltv-demo-mcp.rs the MCP server binary
xtask/                 cargo xtask live-check | capture-fixtures [--trim] | mcp-call (never ships)
```

chromiumoxide is imported by exactly one module. The operations take a `Source`, never a `Session`,
so the whole of `search`/`resolve`/`get` is tested against a fake browser with no Chrome and no
network, and the real browser is exercised only where it must be.

Boundaries:

- **Pure** (`core/`, `browser/arrival.rs`) is where the rules live and where the unit tests point:
  URL handling, page identity, manifest bookkeeping, and every timing rule of a read or a download.
- **Browser** (`browser.rs`) is the only module that knows chromiumoxide. It owns the profile,
  converts CDP errors into domain errors, and exposes `Session::open`, `read`, and the download.
  Every page it opens is closed however the operation ends.
- **HLTV shape** (`hltv/extract.js`) is a set of readers evaluated *inside the page* (and inside a
  saved page in tests), so the same selector code runs live and offline.

How the browser layer works:

- **Navigation is fired, never awaited.** `Page.navigate` runs in a detached task; the read probes
  the page every 250 ms (`location.href`, `document.title`, the selector, `readyState`) and
  `arrival.rs` decides. A page that loaded without the selector is read after 5 s, since HLTV renders
  on the server and an empty results page is complete when it loads. A network error page is
  `Unreachable` at once; a page that loaded somewhere else is `BrowserFailed` after 5 s; a challenge
  title gets 30 s. A navigation that has not committed after 60 s is `Unreachable`.
- **Downloads use `Browser.setDownloadBehavior(allowAndName)` with events.** `downloadWillBegin`
  names the guid and HLTV's file; `downloadProgress` is followed to `completed`; then the guid file
  is renamed. A suggested name that could leave the folder is refused. Sixty seconds without progress
  cancels the download in the browser and fails it. While a demo URL has not turned into a download,
  a document in the page (the challenge, or an HLTV page) is waited on for 30 s, then reported.
- **Launch switches replace chromiumoxide's defaults**, which include `--enable-automation`. The
  keychain switches (`--use-mock-keychain`, `--password-store=basic`) must stay: Chrome encrypts the
  profile's cookies, Cloudflare's clearance among them, with the key they select, so changing them
  loses every cookie an existing profile holds. The TypeScript version launched Chrome the same way.
- **Errors are values.** Every operation returns `Result<_, HltvError>`; nothing is thrown, so an MCP
  client never sees an internal error for a domain failure, and the CLI maps a panic to exit 1 rather
  than Rust's 101. There is no automatic retry: a `Challenge` must not be retried, and retrying a
  download is a user decision, not a policy the tool should hide.
- **Signals belong to the binaries.** Each runs its work where a signal can drop it, which kills
  Chrome (the child is `kill_on_drop`), removes the partial file, and releases the lock. The MCP
  binary shuts its runtime down with a timeout: dropping it would wait forever on the blocking stdin
  read a connected client keeps open. The library installs no handler.

## Testing

1. **Unit (`cargo test --lib`)** — pure modules, no browser: URL classification, page keys, dates,
   manifest round-trip and invalidation, profile lock and Chrome's `SingletonLock`, path
   normalization, the arrival and download state machines, archive names, partial-file cleanup.
2. **Operations (`tests/operations.rs`)** — `search`, `resolve`, and every `get` path (skip, no demo,
   download failure, filter kept, re-fetch after delete) against a fake `Source`.
3. **Fixture (`tests/fixtures.rs`)** — real HTML captured from hltv.org and trimmed, loaded into
   `Offline` (headless, every host unresolvable) and read with the production readers; plus the live
   read path with no network, which must be `Unreachable`, not a challenge. This is the drift alarm
   for HLTV markup. Needs Chrome; without it each test says so and passes.
4. **MCP (`tests/mcp.rs`, `tests/protocol.rs`)** — the tools through rmcp's client over an in-process
   stream and a fake browser (clamps, failures as `{ message, remediation }`, schemas), then the real
   binary as a subprocess: `tools/list`, `tools/call`, error results, and health after an error.
5. **CLI (`tests/cli.rs`)** — the real binary: usage errors exit 2, domain failures exit 1, `--help`
   and no command exit 0, off-site URLs fail before a browser opens, the exact argv Chrome is
   launched with (headed and `--headless`), and a profile held by a run or by another Chrome.
6. **Signals (`tests/signals.rs`)** — a stand-in Chrome that never answers holds a run at launch with
   the lock taken; SIGINT and SIGTERM must kill it, release the lock, and exit 128 + n, for the CLI
   and for an MCP tool call.
7. **Live check (`cargo xtask live-check`)** — the only end-to-end run, and
   it discovers its target instead of naming one: read the newest `/results` page, walk the
   candidates until a match publishes a demo, then assert that match through the CLI (`search`,
   `resolve`, `download`, repeat-skip) and through the MCP server (all four tools, over stdio, through
   rmcp's client), both sharing one folder and one manifest. A hardcoded match URL rots when its demo
   is removed; a discovered one keeps the assertion honest. Needs network and Chrome; exit 2 when a
   prerequisite is missing, which includes Cloudflare's challenge, a held profile, no network, and
   no recent match publishing a demo yet.

Fixtures are trimmed to the subtrees the readers use, as part of capture rather than as a cleanup
step: `cargo xtask capture-fixtures` reads each page through `Session::read` (so a challenge is
reported, never written), prunes to `.results-sublist`, `.result-con`, `.timeAndEvent`, `.streams`,
and the demo anchor, drops scripts, comments, ads and tracking pixels, and reduces every attribute to
an allow-list (`class`, `href`, `src`, `alt`, `title`, `data-unix`, `data-demo-link`,
`data-time-format`). The two files are 57 KB together, down from 746 KB of page dumps, and nothing is
written that still carries `<script`, a comment, a tracker, or a per-visitor attribute — or that
lost the anchor it exists to test. `--trim` re-trims the saved files in `Offline`. The trim is a
rule-for-rule port of the TypeScript one: re-trimming the TypeScript captures is byte-identical.

## Legal

hltv.org's Terms of Use §2.2 prohibit data mining and scraping. This tool fetches the URLs a user
gives it, one match at a time, in the user's own browser session, and refuses to crawl. Keep it that
way when contributing: no unbounded paging, no concurrency by default, no bypass, no bundled
profile. Respect removal requests from the site.

## Decisions

v0.2 (2026-10-25), each a deliberate difference from the TypeScript version it replaced:

- **Rust replaces TypeScript.** The TypeScript version was removed once this one matched it test for
  test and passed the live check. The profile, its lock file, and the manifest format are unchanged,
  so profiles and archive folders it created keep working.
- **A map name that cannot be one is `NotAMap`**, refused before Chrome opens: a usage error on the
  CLI, a tool failure over MCP. TypeScript threw a defect after launching Chrome (open decision D2).
- **Launch failures stay `BrowserMissing`** (open decision D5), with one exception: a profile that
  another Chrome already has open — its `SingletonLock` names a running pid, as the window opened to
  answer the challenge does — is `ProfileBusy`. Same exit code, accurate message.
- **No network is `Unreachable`, exit 2.** TypeScript reported a Cloudflare challenge, whose
  remediation sent the user to answer a challenge that was not there; the CLI contract already
  listed "no network" as a missing prerequisite. A page that went somewhere else is `BrowserFailed`
  naming where, after 5 s, instead of a challenge after 60 s.
- **A challenge gets 30 s, on reads and downloads.** TypeScript failed a read at the first
  challenge title, and waited up to 10 minutes for a download behind one. Thirty seconds lets
  Cloudflare's own check finish and lets a person answer it in the window, and still reports a
  challenge that stays.
- **A loaded page without the selector is read after 5 s**, so an empty results page is
  `NoMatchesFound` in seconds rather than after a 60 s wait for a selector that cannot appear.
- **A download that stops making progress for 60 s fails**, and is cancelled in the browser.
  TypeScript waited on it indefinitely.
- **Signals close Chrome in both binaries**, as Playwright's handlers did, and exit `128 + n`.
- **MCP numbers are integers.** TypeScript's schema said `number`; a fraction is now a parameter
  error, and a negative value reads as 0.
- **Development tooling is `cargo xtask`**, a workspace member that never ships:
  `live-check`, `capture-fixtures`, and `mcp-call`, in place of the TypeScript scripts.

## Verified runs

v0.2 (Rust), 2026-10-25:

| Run | Result |
|---|---|
| `search --team 9565 --map de_cache --limit 3` | 3 matches in 2.2 s: 2399723, 2398757, 2398754, as v0.1 |
| `resolve <match>` | `/download/demo/113159` in 0.9 s |
| `download /download/demo/113159` | 480 MB under HLTV's name, 2 `.dem`; repeat `skip` in 0.37 s |
| the same archive, Rust then TypeScript, back to back | 23.9 s and 38.5 s, byte-identical archives |
| `cargo xtask live-check` | 25/25 in 48 s, target discovered on `/results` (G2 vs NAVI), 703 MB, 2 `.dem`, MCP reusing the CLI's archive |
| SIGINT during a real download | exit 130 at 7 MB; no `.crdownload` left, lock released, no Chrome on the profile |
| `cargo test --workspace` | 97 pass: unit, operations, fixtures, MCP in-process and over stdio, CLI, signals, doctest, xtask |
| `cargo xtask capture-fixtures --trim` | both fixtures byte-identical to the TypeScript capture |
| `cargo install --path .` | installs `hltv-demo` and `hltv-demo-mcp` |

v0.1 (TypeScript), 2026-10-24:

| Run | Result |
|---|---|
| `search --team 9565 --map de_cache --limit 3` | 3 matches in 2.8 s, deduplicated |
| `download <match>` | 480 MB in 7.4 s wall, 2 `.dem` (Mirage, Cache) |
| repeat `download <match>` | `skip`, 0.8 s |
| `download 'results?team=9565&map=de_inferno' --limit 2` | 746 MB + 444 MB in 23 s, the two Vitality matches the filter names |
| first run with a fresh profile whose folder does not exist | 1 match in 2.5 s; it used to fail as `BrowserMissing` ("no Chrome found") |
| `scripts/live-check.sh` | 25/25 in 31 s, target discovered (G2 vs NAVI), 703 MB, 2 `.dem` |
| downstream sanity | `nades ingest` read the archive's map and event from HLTV's file name |

## Open questions

- Does `?event=` filter work on `/results`? Unverified, not implemented.
- Do archived demos disappear from old matches, and after how long? That decides whether `resolve`
  caches negative answers.
- Linux without Chrome on the default path: `CHROME=` is required and untested end to end, as is
  whether a headed Chrome there needs `xvfb`.
- `--headless` exists only to make the failure legible; it is not a supported mode.
- The rest of D5: should a Chrome that will not launch for any other reason be `BrowserFailed`
  (exit 1) rather than `BrowserMissing` (exit 2)?
- A lazy browser (Chrome opened on the first `Source` call) would let a repeat `download` of a demo
  URL skip launching Chrome at all; it changes what users see, so it is still a proposal.
