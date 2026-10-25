# hltv-demo

Find and download professional CS2 demo archives from hltv.org through a real Chrome.

```bash
hltv-demo search   --team 9565 --map de_cache --limit 3   # what exists
hltv-demo resolve  <match-url>                            # does it have a demo
hltv-demo download <url> --out ./demos                    # get it
```

A CLI, a [library](#library), and an [MCP server](#mcp-server) over the same operations. Archives
keep HLTV's own file names (`<event>-<team>-vs-<team>-bo<n>-<id>.rar`), so whatever reads them
downstream needs to know nothing about this tool.

## Install

```bash
bun install
bun run src/cli.ts --help
ln -s "$PWD/src/cli.ts" ~/.local/bin/hltv-demo   # optional: the bare `hltv-demo` name
```

Requires [Bun](https://bun.sh) 1.4+ and Google Chrome; `CHROME=/path/to/chrome` selects another
binary (needed on Linux). Nothing is downloaded for you — the tool drives the Chrome you already
have. The package is **Bun-only**: `exports` are TypeScript sources and the bins run under Bun, so
there is no build step and no Node entry point.

## Chrome, and the Cloudflare challenge

hltv.org answers non-browser clients with a Cloudflare challenge (`403`, `cf-mitigated: challenge`),
and `/download/demo/<id>` refuses anything but a real browser navigation — the same cookies over
HTTP still get the 403. So this drives a **headed** Chrome with a persistent profile. Headless is
challenged even with a warm profile.

The first run may get the challenge. The command prints the fix:

```bash
open -a "Google Chrome" --args --user-data-dir=$HOME/.cache/hltv-demo/chrome https://www.hltv.org
```

Answer it once in that window; later runs reuse the cookie. There is no bypass in this tool, and
none is accepted.

## CLI

```
hltv-demo search   [--team ID] [--map NAME] [--limit N] [--offset N]
hltv-demo resolve  <match-url>
hltv-demo download <url> [--out DIR] [--limit N] [--delay MS]
```

Exit codes: `0` success, `1` domain failure, `2` usage error or missing prerequisite. `--json`
prints one JSON object per line on stdout; without it, human progress goes to stderr.

| Flag | Meaning |
|---|---|
| `--team ID` | HLTV team id, as in `hltv.org/team/9565/...` |
| `--map NAME` | `cache` or `de_cache`; the `de_` prefix is added once |
| `--limit N` | `search`: max matches; `download`: matches a results URL contributes (default 1) |
| `--offset N` | results to skip, for paging (HLTV lists 100 per page) |
| `--out DIR` | download destination (default `~/.cache/hltv-demo/archives`) |
| `--delay MS` | pause between matches in a results run (default 2000) |

```bash
hltv-demo search --team 9565 --map de_cache --limit 3
hltv-demo download https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026
hltv-demo download 'https://www.hltv.org/results?team=9565&map=de_inferno' --limit 2
hltv-demo download /download/demo/113159 --out ./demos
```

Any match, results, or `/download/demo/<id>` URL works. `search` reads `/results` only, newest
first, and drops the duplicate a results page lists in its "featured" block, keeping the copy that
has a date. `resolve` prints the demo URL, or exits `1` with `has no demo download link`. One
archive holds every map of the series (bo3 → 2 `.dem`); they are 440–890 MB.

## MCP server

The same operations, as MCP tools over stdio:

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

Failures return `{ message, remediation }`, and the Cloudflare one names the Chrome profile to open.
A bad URL fails before a browser opens; browser-backed tools launch Chrome for that call and close
it after, so an idle server holds no window. The CLI and the server share `.hltv-demo.tsv`, so an
archive fetched by one is skipped by the other.

```bash
bun run scripts/mcp-call.ts search_matches '{"team":9565,"map":"de_cache","limit":2}'
```

## Behaviour worth knowing

`<out>/.hltv-demo.tsv` maps demo URL → file. Downloading the same demo again prints `skip` and costs
nothing; delete the archive and it becomes fetchable again. A results run continues past a match
with no demo (`nodemo`), and keeps the URL's own `team`/`map`/`offset` filter — a filtered URL
never widens to the front page. Nothing partial is ever recorded: an archive is reported only after
the browser's download event completes and the file is on disk.

One run at a time: the profile is locked by pid, so two concurrent runs fail with
`the Chrome profile … is in use` instead of sharing a browser. `HLTV_DEMO_PROFILE=/another/path`
isolates them.

## Library

```ts
import { Effect } from "effect"
import { get, withSource } from "hltv-demo"

const archives = await Effect.runPromise(
  withSource((source) =>
    get(source, "https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026", {
      out: "./demos",
      limit: 1,
      delayMs: 2000,
      onNote: console.error,
    }),
  ),
)
console.log(archives) // { path, bytes, demoUrl, suggestedName }[]
```

`get` takes a `Source`, never a `Session` — which is why both entry points share it and why the
operations are testable without a browser. `withSource` opens one headed Chrome for the duration of
the work and closes it; it is `Effect.scoped` internally, so do not wrap it again. To drive a
session you already opened, pass `fromSession(session)` instead. `get` fails with a typed error
union: `NotHltvUrl`, `UnsupportedPage`, `Challenge`, `NoDemoFound`, `NoMatchesFound`,
`DownloadFailed`, `BrowserMissing`, `BrowserFailed`, `ProfileBusy`, `ManifestUnreadable`.

## Development

```bash
bun test                 # 58 tests
bun run typecheck        # tsc --noEmit
scripts/live-check.sh    # 25 checks against the newest /results match, both entry points
```

| Suite | What it proves | Needs |
|---|---|---|
| `test/unit/*` | URL rules, dates, manifest, lock, operations against a fake Source | nothing |
| `test/fixture/*` | the extractors still match hltv.org markup | Chrome |
| `test/mcp/*` | tool handlers, and the server over stdio via the official client | nothing |
| `test/cli/*` | exit codes and messages of the real subprocess | nothing |
| `scripts/live-check.sh` | both entry points against a live match, sharing one manifest | network + Chrome |

Fixtures are real HTML **trimmed to the subtrees the extractors read** (57 KB, down from 746 KB),
with no scripts, comments, ads, or tracking. Trimming happens during capture, so a page dump cannot
land in the repo:

```bash
bun run scripts/capture-fixtures.ts          # re-capture (headed Chrome, network)
bun run scripts/capture-fixtures.ts --trim   # re-trim only (headless, offline)
```

When HLTV changes its markup, a fixture test fails instead of a download. Design notes, the
invariants, and the error table are in [SPEC.md](SPEC.md); layout and working rules are in
[AGENTS.md](AGENTS.md).

## Scope and limits

- **Not a crawler.** One URL at a time, a limit on every list, a delay between matches. No "all
  demos of event X".
- **No bypass.** No headless trick, no TLS impersonation, no proxy, no bundled cookies or profile.
- **No parsing, no database.** The output is files.
- `search` cannot tell you whether a match has a demo; that needs `resolve`, which visits the match
  page.
- One browser per command or tool call (~0.6 s each). One long-lived browser would leave a window
  open behind an idle MCP server, which is worse.
- Unverified, and flagged as such in [SPEC.md](SPEC.md): `?event=` filtering, and Linux where
  Chrome is not at the macOS path (`CHROME=` required).

## Legal

hltv.org's Terms of Use §2.2 prohibit data mining and scraping. This tool fetches the URLs you give
it, one match at a time, in your own browser session, and refuses to crawl. Use it for the matches
you are studying, not to mirror the site. [SPEC.md](SPEC.md) records the policy this project intends
to keep.
