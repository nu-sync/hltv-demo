# hltv-demo (Rust)

The production implementation of `hltv-demo`. The project's documentation lives one level up:

- [`../README.md`](../README.md) — install, the CLI, the MCP server, the library
- [`../SPEC.md`](../SPEC.md) — verified facts, invariants, errors, architecture, decisions
- [`../AGENTS.md`](../AGENTS.md) — layout, commands, working rules, testing

```bash
cargo test --workspace                 # 97 tests
cargo clippy --workspace --all-targets
cargo install --path .                 # hltv-demo and hltv-demo-mcp
scripts/live-check.sh                  # 25 live checks (network + Chrome)
cargo xtask capture-fixtures [--trim]  # refresh tests/fixtures/*.html
cargo xtask mcp-call <tool> ['{json}'] # one MCP call over stdio
```
