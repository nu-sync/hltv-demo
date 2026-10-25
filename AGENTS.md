# AGENTS.md

## Intent

`hltv-demo` finds and downloads professional CS2 demo archives from hltv.org. You give it a match page,
a results page, or a `/download/demo/<id>` link; it gives you the archive HLTV publishes for that
match, with HLTV's own file name, in a folder you choose.

It is deliberately generic: no demo parsing, no database, no knowledge of a downstream consumer.
Anything that reads `<event>-<team>-vs-<team>-bo<n>-<id>.rar` can use the output, and the manifest
next to it says which demo URL produced which file.

Three entry points, one core:

- **CLI** — `search`, `resolve`, `download`.
- **MCP server** — the same four operations as MCP tools over stdio (`src/mcp.ts`).
- **Library** — `import { withSource, get } from "hltv-demo"`.

[SPEC.md](SPEC.md) holds the verified facts, the invariants, the error table, and the open questions.
When a decision changes, change the spec in the same commit; it is the record of *why*.

## Out of scope

- **Cloudflare bypass.** No headless trick, TLS impersonation, proxy, or bundled profile. A headed
  Chrome answers the challenge; a person answers it once per profile when it appears.
- **Crawling.** Every list has a limit, requests are sequential with a delay, and there is no
  "download everything for event X". hltv.org's Terms §2.2 forbid scraping.
- **Parsing or storing demos**, and anything that couples to a database.
- **Headless as a supported mode.** `--headless` exists only to make the failure legible.

## User stories

| As a user, I want to… | Entry point |
|---|---|
| See a team's recent matches on a map | `hltv-demo search --team 9565 --map de_cache`, MCP `search_matches` |
| Know whether a match has a demo | `hltv-demo resolve <match-url>`, MCP `resolve_demo` |
| Download a match's demo into a folder | `hltv-demo download <url> --out DIR`, MCP `download_demo` |
| Download the newest N matches of a filtered results page | `hltv-demo download 'https://www.hltv.org/results?team=9565&map=de_inferno' --limit 3` |
| See what a folder already holds | `hltv-demo-mcp` tool `list_downloads`, or read `<out>/.hltv-demo.tsv` |
| Use it from an assistant | register `hltv-demo-mcp` as a stdio MCP server |

## Application flow

```
CLI ─┐
     ├─► search / resolve / get  ─► Source ─► Chrome (CDP) ─► .rar + .hltv-demo.tsv
MCP ─┘        src/hltv/*        src/source.ts   src/browser.ts
```

1. **Validate.** The CLI and every URL-taking MCP tool reject a non-hltv.org URL *before* a browser
   opens (`classifyOrFail`). Bare paths such as `/download/demo/113159` are accepted.
2. **Classify.** `/matches/<id>/…` → one match, `/results?…` → its newest matches (keeping the URL's own
   `team`/`map`/`offset`), `/download/demo/<id>` → the archive.
3. **Open one browser per unit of work.** `withSource` acquires a headed Chrome with the persistent
   profile and the profile lock, and releases both when the work ends. A CLI command is one unit; an
   MCP tool call is one unit.
4. **Read pages.** Every operation gets a fresh page, waits for the URL to change (match and demo URLs
   compared by id) and for the selector it needs, then runs an in-page extractor from
   `src/hltv/extract.ts`.
5. **Download.** Navigate to the demo URL, wait for Playwright's `download` event, `saveAs` into the
   output folder. Completion is the event, never a file appearing on disk.
6. **Record.** `<out>/.hltv-demo.tsv` maps demo URL → file name. A repeat fetch skips it; deleting the
   archive makes it fetchable again. A results run continues past a match with no demo.

## Layout

| Path | Role |
|---|---|
| `SPEC.md` | verified facts, invariants, error table, architecture, open questions |
| `src/core/` | rules and small helpers, no browser and no network: `url`, `dates`, `manifest`, `lock`, `errors`, `types` |
| `src/browser.ts` | the only Playwright module: launch, challenge detection, arrival, download, profile lock |
| `src/source.ts` | the browser boundary (`Source`) and `withSource` |
| `src/classify.ts` | URL parsing as a typed effect failure instead of a throw |
| `src/hltv/extract.ts` | in-page DOM readers (serialized into the page, so they close over nothing) |
| `src/hltv/search.ts`, `resolve.ts`, `get.ts` | the operations, over a `Source` |
| `src/cli.ts` | arguments, formatting, exit codes |
| `src/mcp.ts` | `Tool`/`Toolkit` handlers and the stdio `McpServer` layer |
| `src/index.ts` | public library surface |
| `test/unit/`, `test/mcp/`, `test/cli/`, `test/fixture/` | see Testing |
| `scripts/live-check.sh` | thin shell entry point for the live end-to-end run |
| `scripts/live-check.ts` | the live checks: discovers a target on `/results`, drives both transports |
| `scripts/capture-fixtures.ts` | re-capture and trim the saved HTML the fixture tests read |
| `scripts/mcp-call.ts` | call one MCP tool from a shell |

## Commands

```bash
bun install                                   # dependencies; no browser download (playwright-core drives Chrome)
bun run check                                 # tsc --noEmit && bun test
bun test                                      # 63 tests
bun run src/cli.ts --help
bun run src/cli.ts search --team 9565 --map de_cache --limit 3 --json
bun run src/cli.ts download 'https://www.hltv.org/results?team=9565&map=de_inferno' --limit 2
bun run src/mcp.ts                            # MCP over stdio
bun run scripts/mcp-call.ts search_matches '{"team":9565,"map":"de_cache","limit":2}'
bun run scripts/capture-fixtures.ts           # re-capture + trim the fixtures (headed Chrome)
bun run scripts/capture-fixtures.ts --trim    # re-trim only (headless, offline)
scripts/live-check.sh                         # 25 live checks, needs network + Chrome (~30 s)
```

Exit codes: `0` success, `1` domain failure, `2` usage error or missing prerequisite.

## Working rules

- **Never add a Cloudflare bypass**, a proxy, a bundled cookie or profile, or a headless workaround.
  When a challenge appears, the tool prints the `open -a "Google Chrome" --args --user-data-dir=…` line
  and stops. That is the design, not a gap.
- **The browser is headed, one process per profile.** `src/core/lock.ts` enforces it; do not remove it
  to "fix" a hang. Two runs sharing a profile mix their downloads.
- **Never rename a downloaded archive.** The file name is HLTV's, and downstream tools parse the event
  out of it. It comes from `download.suggestedFilename()`.
- **A results URL keeps its own filter.** `results?team=…&map=…` must never widen to `/results`;
  `resultsFilter` exists because it did once.
- **URL failures are typed, not thrown.** A throw inside an Effect becomes a defect, which an MCP client
  reports as an internal error. Use `classifyOrFail` at every boundary where a URL enters.
- **Keep the rules out of the transports.** New behavior goes in `src/hltv/*` (operations) or
  `src/core/*` (rules), and both the CLI and the MCP server call it. Neither transport may grow its own
  policy.
- **Keep Playwright in `src/browser.ts`.** Nothing else may open a browser or reach the network.
  `src/core/*` holds rules plus the two small filesystem helpers (`manifest`, `lock`); operations take
  a `Source`, never a `Session`, so they can be tested without Chrome.
- **Normalize paths once.** `out` is canonicalized in `get` and in the `list_downloads` handler; macOS
  `TMPDIR` ends in `/`, so unnormalized paths differ by `//`.
- **Never commit** archives (`*.rar`), demos (`*.dem`), manifests, `.live-checks/`, or `node_modules`.
  `.gitignore` covers each of them; if you add a produced artifact, add it there, not to this list.
  Tests use temp folders; `live-check.sh` deletes its archive unless `LIVE_KEEP=1`, and keeps only its
  evidence (`summary.txt`, `evidence.json`) under `.live-checks/`.
- **Do not crawl, and do not raise the limits.** `search` caps a list at `MAX_LIMIT` (100) matches, so
  the library and both transports do (`--offset` pages); `download` takes one match unless `--limit`
  says otherwise, and `get` owns the delay between matches. Politeness is a requirement, not a default.

## Testing

| Suite | What it proves | Needs |
|---|---|---|
| `test/unit/*` | URL rules, dates, manifest, profile lock, and every operation against a fake `Source` | nothing |
| `test/mcp/tools.test.ts` | tool handlers: parameter clamps, failures as `{ message, remediation }` | nothing |
| `test/mcp/protocol.test.ts` | the server speaks MCP over stdio: `tools/list`, `tools/call`, error results | nothing |
| `test/cli/cli.test.ts` | the real subprocess: exit codes, messages, off-site URL refused without a browser | nothing |
| `test/fixture/*` | the production extractors still match saved hltv.org HTML | Chrome |
| `scripts/live-check.sh` | both transports against the newest match on `/results`, sharing one manifest | network + Chrome |

When HLTV's markup changes, a fixture test fails, not a download. Refresh with
`bun run scripts/capture-fixtures.ts` and review the diff: it writes *trimmed* pages (subtrees the
extractors read, attributes allow-listed, no scripts or comments), and refuses to write one that lost its
anchor or is still a challenge page, so a bad capture cannot replace a good fixture.

## Before claiming a change works

```bash
bun run check          # types + 63 tests
scripts/live-check.sh  # 25 checks; exit 2 means a prerequisite is missing (a coverage gap, not a pass)
```

If a change touches the browser layer, selectors, the manifest, or either transport, `live-check.sh` is
required — the offline suites cannot see Cloudflare, markup drift, or a download that never starts.
Record what was measured in `SPEC.md` when a number or a fact changes.

## Notes and known gaps

- Verified live: 480 MB archive in 7.4 s; repeat fetch `skip` in 0.8 s; three matches in 2.8 s;
  `?offset=` paging; the featured duplicate on results pages.
- Not verified: `?event=` filtering (not implemented), Linux with Chrome off the macOS path
  (`CHROME=` required), and registration in a real Claude Desktop config (the SDK client covers the
  wire protocol).
- Demos are 440–890 MB and archives are kept forever. There is no prune command; deleting a file makes
  it fetchable again.
- Environment: Bun 1.4, Google Chrome, `bsdtar` for checking archives. `effect` 4.0.6,
  `@effect/platform-bun` 4.0.6, `playwright-core` 1.64 (no bundled browser), TypeScript 7,
  `@modelcontextprotocol/sdk` 1.32 (dev only, for the protocol test).
