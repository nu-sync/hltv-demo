# hltv-demo — specification

Find and download professional CS2 demo archives from hltv.org through a real browser.

```
                    ┌── CLI (src/cli.ts)
search  filters ──► │                    ──► matches          (what exists)
resolve match   ──► │  shared operations   ──► demo archive URL (does this match have a demo)
download URL    ──► │  src/hltv/*          ──► .rar on disk     (get it)
                    └── MCP server (src/mcp.ts, stdio)
```

Status: v0.1, 2026-10-24. Everything under [Verified facts](#verified-facts) was measured against
live hltv.org; everything else is design. What the shipped v0.1 still has not proven is listed under
[Open questions](#open-questions) — it is not implied by the checks below.

## Purpose

Turn an hltv.org URL, or a filter over recent results, into demo archives on disk. It is a library
plus a CLI, with no knowledge of any downstream consumer: archives keep HLTV's own file names.

## Non-goals

- **No Cloudflare bypass.** No headless trick, no TLS impersonation, no proxy. A real, headed Chrome
  answers the challenge, and a human answers it once per profile when it appears. Shipping a bypass
  is out of scope permanently, not deferred.
- **No crawler.** No "download every demo of event X", no unbounded paging, no background daemon.
  Every list has a limit and there is a delay between requests.
- **No demo parsing, storage service, or database.** The output is files.
- **No downstream integration.** A consumer reads the directory; this tool does not import one.

## Verified facts

Measured on 2026-10-24 on macOS with Google Chrome 155, from the Rust implementation this replaces.

| Fact | Evidence |
|---|---|
| Every non-browser request is challenged: `403`, `cf-mitigated: challenge` | `curl` on `/results`, `/matches/<id>`, `/download/demo/<id>`, even `/rss/news` |
| Headless Chromium is challenged even with a warm profile | `--headless` run against the same profile that passes headed |
| A headed Chrome with a fresh persistent profile passes in ~1.2 s | Playwright, `channel: "chrome"`, `headless: false` |
| `/download/demo/<id>` refuses everything but a browser navigation | `APIRequestContext` with the browser's own cookies and UA, after a successful page load: `403` |
| A match page exposes its demo as `a[href^="/download/demo/"]` | one per match, including matches that later lose it |
| One archive holds every map of the series | Vitality vs MOUZ bo5 → 3 `.dem` (cache, inferno, dust2); bo3 → 2 |
| Archive names are HLTV's own: `<event>-<team>-vs-<team>-bo<n>-<id>.rar` | e.g. `pgl-masters-bucharest-2026-vitality-vs-9z-bo3-eP_cF4LqZBl7PERj7oPEob.rar` |
| Archive sizes are 440–890 MB; download 36–41 s on a fast link | measured over 5 archives |
| Chrome **overwrites** an existing download of the same name (no " (1)") | a repeat navigation to the same demo id rewrote a 703 MB file |
| `Page.navigate`'s reply can arrive long after the page loads, and never arrives for a download | 25–30 s per page in the first implementation |
| A results page shows a match twice: in the "featured" block and again under its date heading | the featured copy has no date heading, so a naive `--limit 1` returns a duplicate |
| `?offset=N` pages the results list | `--offset 1` on Vitality/Cache returned [2398757, 2398754] against [2399723, 2398757] at offset 0 |
| Two runs on one Chrome profile corrupt each other | a backgrounded results run and a foreground demo run shared the profile and downloads crossed over |

## Two entry points, one core

The CLI and the MCP server are transports over the same three operations. Neither contains a rule:

| Layer | Owns |
|---|---|
| `src/core/*` | URL grammar, page identity, dates, manifest, profile lock, the error table |
| `src/browser.ts` | Playwright: headed Chrome, arrival, download completion, profile lock |
| `src/source.ts` | the browser boundary (`Source`) and `withSource`, one browser per unit of work |
| `src/hltv/*` | `search`, `resolve`, `get` over a `Source` |
| `src/cli.ts`, `src/mcp.ts` | arguments, formatting, exit codes / tool results |

Both validate a URL at their own boundary, before a browser opens, so a bad URL is instant in either.

## CLI contract

```
hltv-demo search [--team ID] [--map NAME] [--limit N] [--offset N] [--json]
hltv-demo resolve <match-url> [--json]
hltv-demo download <url> [--out DIR] [--limit N] [--delay MS] [--json]
```

MCP tools (stdio, `bun run src/mcp.ts`): `search_matches {team?, map?, limit?, offset?}`,
`resolve_demo {matchUrl}`, `download_demo {url, out?, limit?, delayMs?}`,
`list_downloads {out?}`. Tool failures carry `{ message, remediation }`; a failure is a tool error
result, never a protocol crash.

- `<url>` is any hltv.org match, results, or `/download/demo/<id>` URL. Bare paths are accepted.
- `--limit` defaults to 1 and caps how many matches a results URL contributes.
- `--out` defaults to `~/.cache/hltv-demo/archives`.
- `--json` prints one JSON object per line and nothing else on stdout; human output goes to stderr.
- Exit codes: `0` success, `1` failure, `2` usage or missing prerequisite (no Chrome, no network).

Examples:

```bash
hltv-demo search --team 9565 --map de_cache --limit 5 --json
hltv-demo resolve https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026
hltv-demo download 'https://www.hltv.org/results?team=9565&map=de_cache' --limit 2
hltv-demo download /download/demo/113159 --out ./demos
```

## Data model

```ts
type Match = {
  matchId: number
  url: string                     // absolute
  date: string | null             // ISO day, from the results list heading
  event: string | null
  teams: [string, string]
  scores: [number, number] | null
}

type DemoRef = { matchId: number; demoId: number; url: string; matchUrl: string }

type Archive = { path: string; bytes: number; demoUrl: string; suggestedName: string }
```

## Invariants

1. **hltv.org only.** Scheme and host are checked before a browser opens; anything else is
   `NotHltvUrl`. Bare paths are resolved against `https://www.hltv.org`.
2. **No archive without a completed download.** The file is recorded only after the browser reports
   the download complete and the file is on disk.
3. **A demo URL is downloaded once** while its file exists. `<out>/.hltv-demo.tsv` maps demo URL →
   file name; a missing file invalidates the entry.
4. **Page identity, not load state.** "Arrived" means the URL matches by match id / demo id (slug and
   query ignored). `readyState` from the previous page is never trusted.
5. **One browser, one process.** A profile lock (pid file) makes a second concurrent run fail with
   `ProfileBusy` instead of sharing the browser and mixing its downloads. No parallel requests.
6. **A results URL keeps its own filter.** `/results?team=9565&map=de_inferno` must never widen to
   `/results`; the query is parsed, not discarded.
7. **A match appears once.** Featured and dated copies of the same match id collapse, keeping the copy
   that carries a date.
8. **One core, two transports.** The CLI and the MCP server call the same operations; a rule added for
   one is a rule for both, and neither may grow its own URL or skip policy.
9. **A URL is validated where it enters.** Both entry points reject a non-hltv.org URL before opening a
   browser, so a typo never costs a Chrome launch.

## Errors

Typed, exhaustive, each with a remediation a user can act on:

| Error | Meaning | Remediation printed |
|---|---|---|
| `NotHltvUrl` | not hltv.org | show the accepted shapes |
| `UnsupportedPage` | hltv.org, but not match/results/demo | point at the three shapes |
| `Challenge` | Cloudflare challenge instead of content | the exact `open -a` command and profile path |
| `NoDemoFound` | match page has no demo link | "the match has no demo (or HLTV changed its markup)" |
| `NoMatchesFound` | results page listed nothing | the filters and the URL |
| `DownloadFailed` | canceled, incomplete, or timed out | what was written so far |
| `BrowserMissing` | no Chrome found | `CHROME=/path/to/chrome` |
| `ManifestUnreadable` | `.hltv-demo.tsv` is corrupt | the file and line |

## Search capability matrix

`search` reads `/results` and nothing else. Honesty about what is proven:

| Filter / behavior | Status |
|---|---|
| `?team=<id>&map=<de_map>` | **verified** — the URL shape the nades `just coverage` links use |
| `?offset=<n>` paging | **verified** — offset 1 skipped one result |
| `?event=<id>` | unverified |
| date-bounded listing (`/stats/matches?startDate=&endDate=`) | out of scope for v0.1; it lists per-map stats pages, a different identity model |
| a match's demo availability | only from the match page: `search` never claims a demo exists |

## Architecture

```
src/core/      url.ts (parse, classify, pageKey) · dates.ts · manifest.ts · lock.ts · errors.ts · types.ts
src/browser.ts Chrome session: launch, challenge detection, navigate-and-arrive, download
src/source.ts  the Source boundary over a session, and withSource (one browser per unit of work)
src/hltv/      extract.ts (in-page DOM readers) · search.ts · resolve.ts · get.ts
src/cli.ts     argument parsing, human output, JSON lines, exit codes
src/mcp.ts     the same operations as four MCP tools over stdio
```

Playwright is imported by exactly one module. The operations take a `Source`, never a `Session`, so
the whole of `search`/`resolve`/`get` is tested against a fake browser with no Chrome and no network,
and the real browser is exercised only where it must be.

Boundaries:

- **Pure** (`core/`) is where the rules live and where the unit tests point: URL handling, page
  identity, manifest bookkeeping. No I/O, no browser.
- **Browser** (`browser.ts`) is the only module that knows Playwright. It owns the profile, converts
  playwright errors into domain errors, and exposes three verbs: `gotoArrived`, `downloadTo`,
  `read`.
- **HLTV shape** (`hltv/extract.ts`) is a set of functions executed *inside the page* (and inside a
  fixture page in tests), so the same selector code runs live and offline.

Effect is used where it earns its place: `Effect.gen` for orchestration, `Data.TaggedError` unions for
the error table, `Effect.scoped`/`Effect.acquireRelease` so the browser and the profile lock are always
released, and `Tool`/`Toolkit`/`McpServer` for the MCP transport. A thrown error would become a defect
(an "internal error" to an MCP client), so `classifyOrFail` turns URL failures into typed ones at both
entry points. There is no automatic retry: a `Challenge` must not be retried, and retrying a
download is a user decision, not a policy the tool should hide.

Playwright supplies what the browser does not: `download.saveAs(path)` picks the output path, so
Chrome's habit of overwriting a same-named download is irrelevant; `waitUntil: "commit"` skips the
navigation reply that never arrives for a download; and a **fresh page per operation** makes the
stale-`readyState` failure impossible rather than merely detected.

## Testing

1. **Unit (`bun test test/unit`)** — pure modules and the operations against a fake `Source`, no
   browser: URL classification, page keys, dates, manifest round-trip and invalidation, profile lock,
   search/dedupe/limit, resolve, and every `get` path (skip, no demo, download failure, filter kept).
2. **Fixture (`bun test test/fixture`)** — real HTML captured from hltv.org and trimmed, loaded into a
   headless page with `setContent`, and read with the production extractors. No network, no challenge. This is
   the drift alarm for HLTV markup: `bun run scripts/capture-fixtures.ts` refreshes the fixtures, and
   a selector change fails a test instead of failing in production.
3. **MCP protocol (`bun test test/mcp`)** — tool handlers with a fake browser, then the real server as a
   subprocess driven by the official MCP client: `tools/list`, `tools/call`, error results, and
   continued health after an error.
4. **CLI (`bun test test/cli`)** — the real subprocess: usage errors exit 2, domain failures exit 1,
   `--help` exits 0, off-site URLs fail before a browser opens.
5. **Live check (`scripts/live-check.sh` → `scripts/live-check.ts`)** — the only end-to-end run, and it
   discovers its target instead of naming one: read the newest `/results` page, walk the candidates
   until a match publishes a demo, then assert that match through the CLI (`search`, `resolve`,
   `download`, repeat-skip) and through the MCP server (all four tools, over stdio, through the official
   client), both sharing one folder and one manifest. A hardcoded match URL rots when its demo is
   removed; a discovered one keeps the assertion honest. Needs network and Chrome; exit 2 when a
   prerequisite is missing, which includes Cloudflare's challenge and a held profile.

Fixtures are trimmed to the subtrees the extractors read, as part of capture rather than as a cleanup
step: `scripts/capture-fixtures.ts` prunes to `.results-sublist`, `.result-con`, `.timeAndEvent`,
`.streams`, and the demo anchor, drops scripts, comments, ads and tracking pixels, and reduces every
attribute to an allow-list (`class`, `href`, `src`, `alt`, `title`, `data-unix`, `data-demo-link`,
`data-time-format`). The two files are 57 KB together, down from 746 KB of page dumps, and the script
refuses to write an output that still carries `<script`, a comment, a tracker, or a per-visitor
attribute — or one that lost the anchor it exists to test, so a Cloudflare challenge page can never
overwrite a fixture. `--trim` re-trims the saved files offline and headless.

## Legal

hltv.org's Terms of Use §2.2 prohibit data mining and scraping. This tool fetches the URLs a user
gives it, one match at a time, in the user's own browser session, and refuses to crawl. Keep it that
way when contributing: no unbounded paging, no concurrency by default, no bypass, no bundled
profile. Respect removal requests from the site.

## Verified runs (v0.1)

| Run | Result |
|---|---|
| `search --team 9565 --map de_cache --limit 3` | 3 matches in 2.8 s, deduplicated |
| `resolve <match>` | `/download/demo/113159` |
| `download <match>` | 480 MB in 7.4 s wall, 2 `.dem` (Mirage, Cache) |
| repeat `download <match>` | `skip`, 0.8 s |
| `download 'results?team=9565&map=de_inferno' --limit 2` | 746 MB + 444 MB in 23 s, the two Vitality matches the filter names |
| `bun test` | 58 pass (units, fixtures, tool handlers, MCP protocol, CLI) |
| `bun run scripts/capture-fixtures.ts --trim` | 205 KB → 54 KB and 541 KB → 3 KB, 5/5 fixture tests still pass |
| `scripts/live-check.sh` | 25/25 in 31 s, target discovered on `/results` (G2 vs NAVI), 703 MB, 2 `.dem`, MCP reusing the CLI's archive |
| downstream sanity | `nades ingest` read the archive's map and event from HLTV's file name |
| MCP `search_matches`, `list_downloads`, `download_demo` | answered over stdio; `download_demo` skipped the archive the CLI had just written |

## Open questions

- Does `?event=` filter work on `/results`? Unverified, not implemented.
- Do archived demos disappear from old matches, and after how long? That decides whether `resolve`
  caches negative answers.
- Linux without Chrome on the default path: `CHROME=` is required and untested end to end.
- `--headless` exists only to make the failure legible; it is not a supported mode.
- Is `channel: "chrome"` enough everywhere, or does Linux need a bundled Chromium plus `xvfb`?
