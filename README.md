# hltv-demo

Find and download professional CS2 demo archives from hltv.org through a real Chrome — as a command
line tool, an MCP server, or a Rust library.

```bash
hltv-demo search   --team 9565 --map de_cache --limit 3   # what exists
hltv-demo resolve  <match-url>                            # does it have a demo
hltv-demo download <url> --out ./demos                    # get it
```

Archives keep HLTV's own file names (`<event>-<team>-vs-<team>-bo<n>-<id>.rar`), so whatever reads
them downstream needs to know nothing about this tool, and a manifest beside them records which demo
URL produced which file.

## Install

```bash
git clone git@github.com:nu-sync/hltv-demo.git && cd hltv-demo
cargo install --path .             # hltv-demo and hltv-demo-mcp, into ~/.cargo/bin
```

Requires Rust 1.88 or newer and Google Chrome. Nothing is downloaded for you: the tool drives the
Chrome you already have, at the macOS install path unless `CHROME=/path/to/chrome` says otherwise.

## Chrome, and the Cloudflare challenge

hltv.org answers non-browser clients with a Cloudflare challenge (`403`, `cf-mitigated: challenge`),
and `/download/demo/<id>` refuses anything but a real browser navigation — the same cookies over
HTTP still get the 403. So this drives a **headed** Chrome with a persistent profile. Headless is
challenged even with a warm profile.

A challenge often clears itself in the headed window within a few seconds, and you can answer one
there yourself; the tool waits 30 s for that. If it is still standing, the command prints the fix:

```bash
open -a "Google Chrome" --args --user-data-dir=$HOME/.cache/hltv-demo/chrome https://www.hltv.org
```

Answer it once in that window, close the window, and run the command again; later runs reuse the
cookie. While that window is open the profile is in use, and the tool says so. There is no bypass in
this tool, and none is accepted.

## Command line

```
hltv-demo search   [--team ID] [--map NAME] [--limit N] [--offset N] [--json]
hltv-demo resolve  <match-url> [--json]
hltv-demo download <url> [--out DIR] [--limit N] [--delay MS] [--json]
```

| Flag | Meaning |
|---|---|
| `--team ID` | HLTV team id, as in `hltv.org/team/9565/vitality` |
| `--map NAME` | `cache` or `de_cache`; the `de_` prefix is added once |
| `--limit N` | `search`: max matches; `download`: matches a results URL contributes (default 1, at most 100) |
| `--offset N` | results to skip, for paging (HLTV lists 100 per page) |
| `--out DIR` | download destination (default `~/.cache/hltv-demo/archives`) |
| `--delay MS` | pause between matches in a results run (default 2000) |
| `--json` | one JSON object per line on stdout, and nothing else there |

```bash
hltv-demo search --team 9565 --map de_cache --limit 3
hltv-demo download https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026
hltv-demo download 'https://www.hltv.org/results?team=9565&map=de_inferno' --limit 2
hltv-demo download /download/demo/113159 --out ./demos
```

Any match, results, or `/download/demo/<id>` URL works, and bare paths are fine. `search` reads
`/results` only, newest first, and drops the duplicate a results page lists in its "featured" block,
keeping the copy that has a date. `resolve` prints the demo URL, or exits `1` with
`has no demo download link`. One archive holds every map of the series (bo3 → 2 `.dem`); they are
440–890 MB.

Exit codes: `0` success, `1` domain failure, `2` usage error or missing prerequisite (no Chrome, a
profile in use, no network), `128 + n` when a signal stopped the run (130 for Ctrl-C) — Chrome is
closed and the profile released first.

| Environment | Meaning |
|---|---|
| `CHROME` | the Chrome binary (default: the macOS install) |
| `HLTV_DEMO_PROFILE` | the persistent profile (default `~/.cache/hltv-demo/chrome`) |
| `HLTV_DEMO_OUT` | the default `--out` |
| `RUST_LOG=hltv_demo=debug` | launch, arrival, and download timings on stderr |

## MCP server

`hltv-demo-mcp` serves the same operations as MCP tools over stdio. Register it with the absolute
path, since desktop apps rarely have `~/.cargo/bin` on their `PATH`:

```json
{
  "mcpServers": {
    "hltv-demo": {
      "command": "/absolute/path/to/.cargo/bin/hltv-demo-mcp"
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

Every tool publishes input and output schemas. Failures are tool errors carrying
`{ message, remediation }`, and the Cloudflare one names the Chrome profile to open. A bad URL or map
name fails before a browser opens; the other tools launch Chrome for that call and close it after, so
an idle server holds no window. The CLI and the server share `.hltv-demo.tsv`, so an archive fetched
by one is skipped by the other.

## Library

```toml
[dependencies]
hltv-demo = { git = "https://github.com/nu-sync/hltv-demo" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use hltv_demo::{Chrome, Options, get, with_source};

#[tokio::main]
async fn main() -> Result<(), hltv_demo::HltvError> {
    let url = "https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026";
    let on_note = |note: &str| eprintln!("{note}");
    let options = Options { out: Some("./demos".into()), on_note: Some(&on_note), ..Default::default() };
    // One headed Chrome for the duration of the work, holding the profile lock, closed after.
    let archives = with_source(&Chrome, async |source| get(source, url, &options).await).await?;
    for archive in archives {
        println!("{} ({} bytes) from {}", archive.path, archive.bytes, archive.demo_url);
    }
    Ok(())
}
```

`get`, `search`, and `resolve` take a `Source`, never a browser: that is why the CLI and the MCP
server share them, and why they are tested without Chrome. `with_source` asks an `Opener` for one
source, runs the work, and closes it however the work ends; `Chrome` is the real one. Everything
fails with `HltvError`, and every variant has a `remediation()`. `cargo doc --open` has the rest.

## Behaviour worth knowing

`<out>/.hltv-demo.tsv` maps demo URL → file. Downloading the same demo again prints `skip` and costs
nothing; delete the archive and it becomes fetchable again. A results run continues past a match
with no demo (`nodemo`), and keeps the URL's own `team`/`map`/`offset` filter — a filtered URL
never widens to the front page. Nothing partial is ever recorded: Chrome writes the download under a
temporary name, and it gets HLTV's name only after the browser reports it complete; an interrupted
run removes it.

One run at a time: the profile is locked by pid, so two concurrent runs fail with
`the Chrome profile … is in use` instead of sharing a browser. `HLTV_DEMO_PROFILE=/another/path`
isolates them.

## Development

```bash
cargo test --workspace                   # 98 tests; the fixture tests need Chrome
cargo clippy --workspace --all-targets   # no warnings
cargo xtask live-check                   # 25 checks against the newest /results match, both entry points
cargo xtask capture-fixtures [--trim]    # refresh tests/fixtures/*.html from hltv.org (or re-trim offline)
cargo xtask mcp-call search_matches '{"team":9565,"map":"de_cache","limit":2}'
```

| Path | What lives there |
|---|---|
| `src/core/` | the rules: URLs, page identity, dates, manifest, profile lock, errors, types |
| `src/hltv/` | `search`, `resolve`, `get`, and the readers that run inside the page |
| `src/browser.rs` | the only code that drives Chrome (CDP, via chromiumoxide) |
| `src/cli.rs`, `src/mcp.rs` | the two transports; `src/main.rs` and `src/bin/` are their binaries |
| `tests/` | operations against a fake browser, MCP over stdio, the CLI, signals, saved HLTV pages |
| `xtask/` | development tasks; never shipped |

When HLTV changes its markup, a fixture test fails instead of a download. The verified facts, the
invariants, the error table, and the reasoning behind each decision are in [SPEC.md](SPEC.md);
working rules for contributors and agents are in [AGENTS.md](AGENTS.md); releases are in
[CHANGELOG.md](CHANGELOG.md).

## Scope and limits

- **Not a crawler.** One URL at a time, a limit on every list, a delay between matches. No "all
  demos of event X".
- **No bypass.** No headless trick, no TLS impersonation, no proxy, no bundled cookies or profile.
- **No parsing, no database.** The output is files.
- `search` cannot tell you whether a match has a demo; that needs `resolve`, which visits the match
  page.
- One browser per command or tool call (~0.3–0.4 s to launch). One long-lived browser would leave a
  window open behind an idle MCP server, which is worse.
- Unverified, and flagged as such in [SPEC.md](SPEC.md): `?event=` filtering, and Linux where
  Chrome is not at the macOS path (`CHROME=` required).

## Legal

hltv.org's Terms of Use §2.2 prohibit data mining and scraping. This tool fetches the URLs you give
it, one match at a time, in your own browser session, and refuses to crawl. Use it for the matches
you are studying, not to mirror the site. [SPEC.md](SPEC.md) records the policy this project intends
to keep.
