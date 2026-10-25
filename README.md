# hltv-demo

Find and download professional CS2 demo archives from hltv.org, through a real browser.

```
hltv-demo search   --team 9565 --map de_cache --limit 3     # what exists
hltv-demo resolve  <match-url>                             # does it have a demo
hltv-demo download <url> --out ./demos                     # get it
```

It is a library, a CLI, and an **MCP server** — all three run the same operations. It knows nothing
about any downstream consumer: archives keep HLTV's own file names, so anything that reads
`<event>-<team>-vs-<team>-bo<n>-<id>.rar` can pick them up.

## Why a browser

hltv.org answers every non-browser client with a Cloudflare challenge (`403`, `cf-mitigated:
challenge`), and `/download/demo/<id>` refuses everything but a real browser *navigation* — the same
cookies through an HTTP client still get the 403. So this drives an actual, **headed** Chrome with a
persistent profile. Headless Chrome is challenged even when the profile is warm; do not plan around it.

The first run may get the challenge. The command prints what to do:

```bash
open -a "Google Chrome" --args --user-data-dir=$HOME/.cache/hltv-demo/chrome https://www.hltv.org
```

Answer it once in that window; later runs reuse the cookie. There is no bypass in this tool and none
will be accepted.

## Install

```bash
bun install
```

Requirements: [Bun](https://bun.sh) 1.2+, and Google Chrome. `CHROME=/path/to/chrome` picks a different
binary (required on Linux). No browser download: the tool drives the Chrome you already have.

```bash
bun run src/cli.ts --help
ln -s "$PWD/src/cli.ts" ~/.local/bin/hltv-demo   # optional
```

## Commands

### search

```bash
hltv-demo search --team 9565 --map de_cache --limit 5
hltv-demo search --map inferno --limit 3 --json
hltv-demo search --team 9565 --offset 100 --limit 100 --json
```

| Flag | Meaning |
|---|---|
| `--team ID` | HLTV team id, as in `hltv.org/team/9565/...` |
| `--map NAME` | `cache` or `de_cache`; the `de_` prefix is added once |
| `--limit N` | max matches (default 1) |
| `--offset N` | results to skip, for paging (verified; HLTV lists 100 per page) |
| `--json` | one JSON object per line on stdout |

`search` only reads `/results`. A results page shows a match twice — once in the "featured" block, once
under its date heading — so results are deduplicated by match id, keeping the copy that has a date.

### resolve

```bash
hltv-demo resolve https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026
# https://www.hltv.org/download/demo/113159
```

Prints the demo URL, or exits 1 with `has no demo download link` when the match publishes none.

### download

```bash
hltv-demo download https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026
hltv-demo download 'https://www.hltv.org/results?team=9565&map=de_inferno' --limit 2
hltv-demo download /download/demo/113159 --out ./demos
hltv-demo download <url> --json        # { "path", "bytes", "demoUrl", "suggestedName" }
```

| Flag | Meaning |
|---|---|
| `--out DIR` | where archives go (default `~/.cache/hltv-demo/archives`) |
| `--limit N` | matches a results URL contributes (default 1) |
| `--delay MS` | pause between matches (default 2000) |
| `--json` | one JSON object per line on stdout, progress on stderr |

One archive holds every map of the series (a bo3 → 2 `.dem`, a bo5 → 3). Sizes are 440–890 MB.

## MCP server

The same operations are exposed as MCP tools over stdio, for Claude Desktop, Claude Code, or any MCP
client. `src/cli.ts` and `src/mcp.ts` differ only in transport and argument shape: both call
`search`, `resolve`, and `get` from `src/hltv/*` through the same `Source` boundary.

```json
{
  "mcpServers": {
    "hltv-demo": {
      "command": "bun",
      "args": ["run", "/absolute/path/to/hltv-demo/src/mcp.ts"]
    }
  }
}
```

| Tool | Arguments | Returns |
|---|---|---|
| `search_matches` | `team?`, `map?`, `limit?` (1–100), `offset?` | recent matches, newest first |
| `resolve_demo` | `matchUrl` | `{ matchId, demoId, url, matchUrl }` |
| `download_demo` | `url`, `out?`, `limit?`, `delayMs?` | `[{ path, bytes, demoUrl, suggestedName }]` |
| `list_downloads` | `out?` | `{ manifest, entries }` |

- Failures come back as tool errors carrying `{ message, remediation }` — e.g. the Cloudflare
  challenge answer tells the user exactly which Chrome profile to open.
- A bad URL fails before a browser opens. Browser-backed tools launch Chrome for that call and close
  it after, so an idle MCP server holds no window.
- The CLI and the MCP server share `.hltv-demo.tsv`, so an archive downloaded by one is skipped by the
  other.
- `HLTV_DEMO_HEADLESS=1` is only for showing that headless is challenged.

Check it by hand:

```bash
bun run scripts/mcp-call.ts search_matches '{"team":9565,"map":"de_cache","limit":2}'
bun run scripts/mcp-call.ts list_downloads '{"out":"/tmp/demos"}'
```

## Architecture

```
src/core/     url · dates · manifest · lock · errors · types        pure rules, no I/O
src/browser.ts   the only Playwright module: headed Chrome, arrival, downloads, profile lock
src/source.ts    the browser boundary (Source) + withSource, used by the CLI and the MCP server
src/hltv/     extract (in-page readers) · search · resolve · get    operations over a Source
src/cli.ts    search | resolve | download; --json; exit 0/1/2
src/mcp.ts    the same four operations as MCP tools, over stdio
```

`search`, `resolve`, and `get` take a `Source`, never a `Session`, which is why they can be tested
against a fake browser and why both entry points share them verbatim.

## Idempotency

`<out>/.hltv-demo.tsv` records demo URL → file. Downloading a demo again prints `skip` and costs
nothing; delete the archive and it becomes fetchable again. In a results run, a match with no demo
prints `nodemo` and the run continues, and the URL's own `team`/`map`/`offset` filter is kept — a
filtered results URL never widens to the front page.

One run at a time: the profile is locked by pid, so two concurrent runs fail with `the Chrome profile
… is in use` instead of sharing a browser. Use `HLTV_DEMO_PROFILE=/another/path` to isolate them.

Nothing partial is ever recorded or returned: an archive is reported only after the browser's download
event completes and the file is on disk.

## Library

```ts
import { Effect } from "effect"
import { Session, get } from "hltv-demo"

const program = Effect.scoped(
  Effect.gen(function* () {
    const session = yield* Session.open()
    return yield* get(session, url, { out: "./demos", limit: 1, delayMs: 2000, onNote: console.error })
  }),
)
await Effect.runPromise(program)
```

`withSource` opens a browser for the duration of the work and closes it (`Effect.acquireRelease`). `get` returns `Archive[]` and
fails with a typed error union: `NotHltvUrl`, `UnsupportedPage`, `Challenge`, `NoDemoFound`,
`NoMatchesFound`, `DownloadFailed`, `BrowserMissing`, `BrowserFailed`, `ManifestUnreadable`.

## Test

```bash
bun test                 # 58 tests: rules, fixtures, tool handlers, MCP protocol, CLI exit codes
bun x tsc --noEmit       # types
scripts/live-check.sh    # 27 checks: search, resolve, download, skip, then the same over MCP
```

What is covered where:

| Suite | What it proves | Needs |
|---|---|---|
| `test/unit/*` | URL rules, dates, manifest, profile lock, and the operations against a fake Source | nothing |
| `test/fixture/*` | the real extractors still match hltv.org markup (saved HTML, headless) | Chrome |
| `test/mcp/tools.test.ts` | tool handlers: parameter clamps, typed failures, manifest sharing | nothing |
| `test/mcp/protocol.test.ts` | the server speaks MCP over stdio: `tools/list`, `tools/call`, error results | nothing |
| `test/cli/cli.test.ts` | exit codes and messages of the real subprocess | nothing |
| `scripts/live-check.sh` | both entry points against live hltv.org, sharing one manifest | network + Chrome |

The fixture tests run the production extractors over real HTML saved from hltv.org, in headless Chrome
with no network. Refresh the fixtures when HLTV changes:

```bash
bun run scripts/capture-fixtures.ts
```

That is the alarm for selector drift: a markup change fails a test instead of a download.

## Scope and limits

- **Not a crawler.** One URL at a time, a limit on every list, a delay between matches. No "all demos
  of event X".
- **No bypass.** No headless trick, no TLS impersonation, no proxy, no bundled cookies or profile.
- **No parsing, no database.** The output is files.
- `search` reads results pages only. It cannot tell you whether a match has a demo without visiting the
  match page (`resolve`).
- One browser per command or tool call: a `search` followed by a `download` launches Chrome twice
  (~0.6 s each). The alternative — one long-lived browser — leaves a window open behind an idle MCP
  server, which was worse.
- Unverified, and flagged as such in [SPEC.md](SPEC.md): `?event=` filtering, and Linux where Chrome
  is not at the macOS path (`CHROME=` required).

## Legal

hltv.org's Terms of Use §2.2 prohibit data mining and scraping. This tool fetches the URLs you give it,
one match at a time, in your own browser session. Use it for the matches you are studying, not to
mirror the site. [SPEC.md](SPEC.md) records the policy this project intends to keep.
