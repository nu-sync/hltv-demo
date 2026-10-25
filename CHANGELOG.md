# Changelog

Notable changes to hltv-demo. The reasoning behind each is in [SPEC.md](SPEC.md), under Decisions.

## 0.2.0 — 2026-10-25

Rewritten in Rust; the TypeScript implementation (0.1.0) is retired. The CLI, the MCP tools, the
JSON output, the Chrome profile, and the manifest format are unchanged, so existing profiles and
archive folders keep working.

- Installs with `cargo install --path .` as two binaries, `hltv-demo` and `hltv-demo-mcp`.
- No network is reported as `Unreachable` (exit 2), not as a Cloudflare challenge.
- A challenge gets 30 s to clear before it is reported, on reads and on downloads.
- An empty results page fails in seconds instead of after a 60 s wait for its selector.
- A download that makes no progress for 60 s fails and is cancelled in the browser.
- A map name that cannot be one is refused before Chrome opens (`NotAMap`).
- A profile already open in another Chrome window is reported as in use, not as a missing Chrome.
- SIGINT, SIGTERM, and SIGHUP close Chrome and release the profile lock; the exit code is 128 + n.
- `RUST_LOG=hltv_demo=debug` prints launch, arrival, and download timings on stderr.
- MCP numbers are integers; a negative `limit`, `offset`, or `delayMs` reads as 0.

## 0.1.0 — 2026-10-24

The first version, in TypeScript on Bun: the CLI, the MCP server over stdio, and the library.
