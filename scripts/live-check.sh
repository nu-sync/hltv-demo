#!/usr/bin/env bash
# Prove the live path still works: search, resolve, download one match, and skip the repeat.
#
# Usage: scripts/live-check.sh [MATCH_URL]
#   MATCH_URL  an hltv.org match page (default: Vitality vs 9z, PGL Masters Bucharest 2026)
# Environment:
#   CHROME      Chrome binary (default: the macOS install)
#   EVIDENCE    where the run logs go (default: <repo>/.live-checks)
# Exit status: 0 every check passed, 1 a check failed, 2 a prerequisite is missing (a coverage gap).

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MATCH="${1:-https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026}"
RUN="$(date +%Y%m%d-%H%M%S)-$$"
EVIDENCE="${EVIDENCE:-$REPO/.live-checks}/$RUN"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/hltv-demo-live.XXXXXX")"
OUT="$(cd "$WORK" && pwd)/archives"
FAILS=0
CHECKS=0

mkdir -p "$EVIDENCE" "$OUT"
SUMMARY="$EVIDENCE/summary.txt"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

log() { printf '%s\n' "$*" | tee -a "$SUMMARY"; }
pass() { CHECKS=$((CHECKS + 1)); log "PASS  $*"; }
fail() { CHECKS=$((CHECKS + 1)); FAILS=$((FAILS + 1)); log "FAIL  $*"; }
gap() { log "GAP   $*"; log "exit 2: a prerequisite is missing, so this is a coverage gap, not a pass"; exit 2; }
# check NAME COMMAND [ARG...] — passes when the command exits 0.
check() { local name="$1"; shift; if "$@" >/dev/null 2>&1; then pass "$name"; else fail "$name"; fi; }
show() { sed 's/^/      /' "$1" >>"$SUMMARY"; }
cli() { (cd "$REPO" && bun run src/cli.ts "$@"); }
lines() { wc -l <"$1" | tr -d ' '; }

log "hltv-demo live check $RUN"
log "match: $MATCH"
log "evidence: $EVIDENCE"

log "== prerequisites"
command -v bun >/dev/null || gap "bun is not installed"
CHROME_BIN="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
[ -x "$CHROME_BIN" ] || gap "no Chrome at $CHROME_BIN (set CHROME)"
command -v bsdtar >/dev/null || gap "bsdtar not found"
[ -d "$REPO/node_modules" ] || gap "dependencies are not installed (bun install)"
pass "bun, Chrome, bsdtar, and node_modules"

log "== unit and fixture tests"
(cd "$REPO" && bun test >"$EVIDENCE/tests.log" 2>&1)
check "bun test passes" grep -q "0 fail" "$EVIDENCE/tests.log"
grep -E "^ *[0-9]+ pass" "$EVIDENCE/tests.log" | sed 's/^/      /' >>"$SUMMARY"

log "== search"
cli search --team 9565 --map de_cache --limit 3 --json >"$EVIDENCE/search.log" 2>&1
check "search exits 0" test -s "$EVIDENCE/search.log"
show "$EVIDENCE/search.log"
check "search returns 3 matches (got $(lines "$EVIDENCE/search.log"))" test "$(lines "$EVIDENCE/search.log")" -eq 3
check "every match id is unique" test "$(grep -o '"matchId":[0-9]*' "$EVIDENCE/search.log" | sort -u | wc -l | tr -d ' ')" -eq 3
check "every match is an hltv.org match URL" grep -q '"url":"https://www.hltv.org/matches/' "$EVIDENCE/search.log"

log "== resolve"
DEMO_URL="$(cli resolve "$MATCH" 2>"$EVIDENCE/resolve.log")"
RC=$?
log "      $DEMO_URL"
check "resolve exits 0" test "$RC" -eq 0
check "resolve returns a demo URL" grep -qE '^https://www\.hltv\.org/download/demo/[0-9]+$' <<<"$DEMO_URL"

log "== download"
cli download "$MATCH" --out "$OUT" >"$EVIDENCE/download.log" 2>&1
check "download exits 0" test $? -eq 0
show "$EVIDENCE/download.log"
ARCHIVE="$(find "$OUT" -maxdepth 1 -name '*.rar' -print -quit)"
check "an archive is on disk" test -n "$ARCHIVE"
BYTES="$(stat -f %z "$ARCHIVE" 2>/dev/null || stat -c %s "$ARCHIVE" 2>/dev/null || echo 0)"
check "the archive is a real download (${BYTES} bytes > 100 MB)" test "${BYTES:-0}" -gt 100000000
check "the archive name is HLTV's (contains -vs-)" grep -q -- "-vs-" <<<"$ARCHIVE"
DEMS="$(bsdtar -tf "$ARCHIVE" 2>/dev/null | grep -c '\.dem$')"
check "the archive holds .dem files (got $DEMS)" test "${DEMS:-0}" -ge 1
check "the download was recorded in the manifest" grep -qF "$DEMO_URL" "$OUT/.hltv-demo.tsv"

log "== repeat download"
SECONDS=0
cli download "$MATCH" --out "$OUT" >"$EVIDENCE/download-2.log" 2>&1
RC=$?
ELAPSED=$SECONDS
show "$EVIDENCE/download-2.log"
check "the repeat exits 0" test "$RC" -eq 0
check "the repeat skips the download" grep -q "skip " "$EVIDENCE/download-2.log"
check "the repeat is fast (${ELAPSED}s <= 10s)" test "$ELAPSED" -le 10
check "no second archive appeared" test "$(find "$OUT" -maxdepth 1 -name '*.rar' | wc -l | tr -d ' ')" -eq 1

log "== mcp over stdio"
(cd "$REPO" && bun run scripts/mcp-call.ts list_downloads "{\"out\":\"$OUT\"}" >"$EVIDENCE/mcp-list.log" 2>&1)
RC=$?
show "$EVIDENCE/mcp-list.log"
check "list_downloads over MCP exits 0" test "$RC" -eq 0
check "it sees the archive the CLI downloaded" grep -qF "$(basename "$ARCHIVE")" "$EVIDENCE/mcp-list.log"

(cd "$REPO" && bun run scripts/mcp-call.ts search_matches '{"team":9565,"map":"de_cache","limit":1}' >"$EVIDENCE/mcp-search.log" 2>&1)
RC=$?
show "$EVIDENCE/mcp-search.log"
check "search_matches over MCP exits 0" test "$RC" -eq 0
check "it returns a match id" grep -q '"matchId"' "$EVIDENCE/mcp-search.log"

(cd "$REPO" && bun run scripts/mcp-call.ts download_demo "{\"url\":\"$MATCH\",\"out\":\"$OUT\"}" >"$EVIDENCE/mcp-download.log" 2>&1)
RC=$?
show "$EVIDENCE/mcp-download.log"
check "download_demo over MCP exits 0" test "$RC" -eq 0
check "it shared the CLI's manifest (no second download)" grep -q "skip " "$EVIDENCE/mcp-download.log"
check "it returned the archive the CLI wrote" grep -qF "$(basename "$ARCHIVE")" "$EVIDENCE/mcp-download.log"

(cd "$REPO" && bun run scripts/mcp-call.ts resolve_demo '{"matchUrl":"https://example.com/x"}' >"$EVIDENCE/mcp-error.log" 2>&1)
RC=$?
show "$EVIDENCE/mcp-error.log"
check "a bad URL over MCP is an error result (exit 1)" test "$RC" -eq 1
check "that error explains itself" grep -q "not an hltv.org URL" "$EVIDENCE/mcp-error.log"

log "== done: $FAILS failed check(s) of $CHECKS"
[ "$FAILS" -eq 0 ] || exit 1
exit 0
