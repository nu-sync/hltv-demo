# AGENTS.md

## Intent

`hltv-demo` finds and downloads professional CS2 demo archives from hltv.org. You give it a match page,
a results page, or a `/download/demo/<id>` link; it gives you the archive HLTV publishes for that
match, with HLTV's own file name, in a folder you choose.

It is deliberately generic: no demo parsing, no database, no knowledge of a downstream consumer.
Anything that reads `<event>-<team>-vs-<team>-bo<n>-<id>.rar` can use the output, and the manifest
next to it says which demo URL produced which file.

Three entry points, one core:

- **CLI** — `hltv-demo search | resolve | download` (`src/cli.rs`, clap).
- **MCP server** — `hltv-demo-mcp`, the same operations as four MCP tools over stdio (`src/mcp.rs`, rmcp).
- **Library** — `use hltv_demo::{with_source, get, Chrome}`.

[SPEC.md](SPEC.md) holds the verified facts, the invariants, the error table, the decisions, and the
open questions. When a decision changes, change the spec in the same commit; it is the record of *why*.

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
| See what a folder already holds | MCP `list_downloads`, or read `<out>/.hltv-demo.tsv` |
| Use it from an assistant | register `hltv-demo-mcp` as a stdio MCP server |

## Application flow

```
CLI ─┐
     ├─► search / resolve / get  ─► Source ─► Chrome (CDP) ─► .rar + .hltv-demo.tsv
MCP ─┘        src/hltv/*         src/source.rs  src/browser.rs
```

1. **Validate.** The CLI and every URL-taking MCP tool reject a non-hltv.org URL, and a map name
   that cannot be one, *before* a browser opens (`classify`, `map_filter`). Bare paths such as
   `/download/demo/113159` are accepted.
2. **Classify.** `/matches/<id>/…` → one match, `/results?…` → its newest matches (keeping the URL's own
   `team`/`map`/`offset`), `/download/demo/<id>` → the archive.
3. **Open one browser per unit of work.** `with_source` asks an `Opener` (`Chrome` in production) for
   a headed Chrome on the persistent profile, holding the profile lock, and closes both when the work
   ends. A CLI command is one unit; an MCP tool call is one unit.
4. **Read pages.** Every operation gets a fresh page; navigation is fired, then the page is probed
   until `browser/arrival.rs` says it arrived (match and demo URLs compared by id) and the selector
   exists or the page settled; then a reader from `src/hltv/extract.js` runs in the page.
5. **Download.** Navigate to the demo URL, wait for Chrome's `downloadWillBegin`, follow
   `downloadProgress` to `completed`, then rename the guid file to HLTV's name. Completion is the
   event, never a file appearing on disk.
6. **Record.** `<out>/.hltv-demo.tsv` maps demo URL → file name. A repeat fetch skips it; deleting the
   archive makes it fetchable again. A results run continues past a match with no demo.

## Layout

| Path | Role |
|---|---|
| `SPEC.md` | verified facts, invariants, error table, architecture, decisions, open questions |
| `src/core/` | rules and small helpers, no browser and no network: `url`, `dates`, `manifest`, `lock`, `errors`, `types` |
| `src/browser.rs` | the only chromiumoxide module: launch, `Session`, `Offline`, page reads, download |
| `src/browser/arrival.rs` | what one probe of a page means (wait, read, fail), with no browser |
| `src/source.rs` | the browser boundary (`Source`, `Opener`) and `with_source` |
| `src/hltv/extract.rs`, `extract.js` | in-page readers (evaluated in the page, so they close over nothing) |
| `src/hltv/search.rs`, `resolve.rs`, `get.rs` | the operations, over a `Source` |
| `src/cli.rs` | arguments, formatting, exit codes |
| `src/mcp.rs` | the rmcp tool router and the stdio server |
| `src/process.rs` | stderr logging and shutdown signals, for the binaries only |
| `src/bin/` | the two binaries, each a few lines |
| `src/lib.rs` | public library surface |
| `tests/` | see Testing; `tests/fixtures/*.html` are the saved hltv.org pages |
| `xtask/` | `cargo xtask live-check`, `capture-fixtures [--trim]`, `mcp-call` (never ships) |
| `scripts/live-check.sh` | thin shell entry point for the live end-to-end run |

## Commands

```bash
cargo build                                   # no browser download: chromiumoxide drives the installed Chrome
cargo test --workspace                        # 97 tests
cargo clippy --workspace --all-targets        # no warnings
cargo fmt --all
cargo run --bin hltv-demo -- --help
cargo run --bin hltv-demo -- search --team 9565 --map de_cache --limit 3 --json
cargo run --bin hltv-demo -- download 'https://www.hltv.org/results?team=9565&map=de_inferno' --limit 2
cargo run --bin hltv-demo-mcp                 # MCP over stdio
cargo install --path .                        # hltv-demo and hltv-demo-mcp into ~/.cargo/bin
cargo xtask mcp-call search_matches '{"team":9565,"map":"de_cache","limit":2}'
cargo xtask capture-fixtures                  # re-capture + trim the fixtures (headed Chrome)
cargo xtask capture-fixtures --trim           # re-trim only (headless, no network)
scripts/live-check.sh                         # 25 live checks, needs network + Chrome (~50 s)
RUST_LOG=hltv_demo=debug cargo run --bin hltv-demo -- ...   # launch, arrival, download timings
```

Exit codes: `0` success, `1` domain failure, `2` usage error or missing prerequisite, `128 + n` when
signal `n` stopped the run.

## Working rules

- **Never add a Cloudflare bypass**, a proxy, a bundled cookie or profile, or a headless workaround.
  When a challenge stands for 30 s, the tool prints the `open -a "Google Chrome" --args --user-data-dir=…`
  line and stops. That is the design, not a gap.
- **The browser is headed, one process per profile.** `src/core/lock.rs` enforces it; do not remove it
  to "fix" a hang. Two runs sharing a profile mix their downloads.
- **Keep the keychain switches.** `--use-mock-keychain` and `--password-store=basic` decide how
  Chrome encrypts the profile's cookies; changing them loses every cookie an existing profile holds,
  Cloudflare's clearance included. chromiumoxide adds the `--` itself: pass `"no-first-run"`, never
  `"--no-first-run"`.
- **Never rename a downloaded archive.** The file name is HLTV's, and downstream tools parse the event
  out of it. It comes from `downloadWillBegin`'s `suggested_filename`.
- **A results URL keeps its own filter.** `results?team=…&map=…` must never widen to `/results`;
  `results_filter` exists because it did once.
- **Errors are values.** Return `HltvError`; never `unwrap` or `panic!` on anything a user or a page
  controls. A panic is an MCP client's "internal error" and the CLI's exit 1.
- **Keep the rules out of the transports.** New behavior goes in `src/hltv/*` (operations) or
  `src/core/*` (rules), and both the CLI and the MCP server call it. Neither transport may grow its own
  policy; they only convert arguments.
- **Keep chromiumoxide in `src/browser.rs`.** Nothing else may open a browser or reach the network.
  Timing decisions go in `src/browser/arrival.rs`, which holds no browser and is unit-tested.
  Operations take a `Source`, never a `Session`, so they can be tested without Chrome.
- **Signals belong to the binaries.** The library installs no handler; the binaries drop their work
  on SIGINT/SIGTERM/SIGHUP so Chrome is killed and the lock released. Keep `hltv-demo-mcp`'s runtime
  `shutdown_timeout`: dropping the runtime would wait forever on the stdin read.
- **Normalize paths once.** `out` is canonicalized by `canonical_out` in `get` and in the
  `list_downloads` handler; macOS `TMPDIR` ends in `/`, so unnormalized paths differ by `//`.
- **Never commit** archives (`*.rar`), demos (`*.dem`), manifests, `.live-checks/`, or `target/`.
  `.gitignore` covers each of them; if you add a produced artifact, add it there, not
  to this list. Tests use temp folders; the live check deletes its archive unless `LIVE_KEEP=1`, and
  keeps only its evidence (`summary.txt`, `evidence.json`) under `.live-checks/`.
- **Do not crawl, and do not raise the limits.** `search` caps a list at `MAX_LIMIT` (100) matches, so
  the library and both transports do (`--offset` pages); `download` takes one match unless `--limit`
  says otherwise, and `get` owns the delay between matches. Politeness is a requirement, not a default.

## Testing

| Suite | What it proves | Needs |
|---|---|---|
| `src/**` unit tests | URL rules, dates, manifest, profile lock, arrival and download timing, paths | nothing |
| `tests/operations.rs` | every operation against a fake `Source` | nothing |
| `tests/mcp.rs` | tool handlers through rmcp's client: clamps, failures as `{ message, remediation }`, schemas | nothing |
| `tests/protocol.rs` | the real `hltv-demo-mcp` speaks MCP over stdio: `tools/list`, `tools/call`, error results | nothing |
| `tests/cli.rs` | the real binary: exit codes, messages, Chrome's argv, held profiles | nothing |
| `tests/signals.rs` | a signal kills Chrome, releases the lock, exits 128 + n, in both binaries | nothing |
| `tests/fixtures.rs` | the production readers still match saved hltv.org HTML; no network is `Unreachable` | Chrome |
| `scripts/live-check.sh` | both transports against the newest match on `/results`, sharing one manifest | network + Chrome |

When HLTV's markup changes, a fixture test fails, not a download. Refresh with
`cargo xtask capture-fixtures` and review the diff: it writes *trimmed* pages (subtrees the readers
use, attributes allow-listed, no scripts or comments), and refuses to write one that lost its anchor
or is still a challenge page, so a bad capture cannot replace a good fixture.

## Before claiming a change works

```bash
cargo test --workspace                   # 97 tests
cargo clippy --workspace --all-targets   # no warnings
scripts/live-check.sh                    # 25 checks; exit 2 means a prerequisite is missing (a coverage gap, not a pass)
```

If a change touches the browser layer, the readers, the manifest, or either transport,
`live-check.sh` is required — the offline suites cannot see Cloudflare, markup drift, or a download
that never starts. Record what was measured in `SPEC.md` when a number or a fact changes.

## Notes and known gaps

- Verified live (2026-10-25): 25/25 live checks; 703 MB archive in 27.8 s of which 1.6 s is the
  tool's; repeat fetch `skip` in 0.37 s; the same archive as TypeScript, byte for byte; a SIGINT
  mid-download leaves nothing behind.
- Not verified: `?event=` filtering (not implemented), Linux with Chrome off the macOS path
  (`CHROME=` required), and registration in a real Claude Desktop config (rmcp's client covers the
  wire protocol).
- Demos are 440–890 MB and archives are kept forever. There is no prune command; deleting a file makes
  it fetchable again.
- Environment: Rust 1.95 (what this is built and tested with; rmcp needs 1.88+), Google Chrome, and
  `bsdtar` for the live check. `chromiumoxide` 0.9 (CDP, no bundled browser), `rmcp` 3.7, `clap` 4.6,
  `tokio` 1.54.
