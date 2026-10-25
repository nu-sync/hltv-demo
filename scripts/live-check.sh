#!/usr/bin/env bash
# Prove the live path still works, for both entry points, against whatever is newest on
# https://www.hltv.org/results. The target is discovered rather than hardcoded, so this keeps working
# when today's match loses its demo. The checks live in scripts/live-check.ts.
#
# Usage: scripts/live-check.sh
# Environment:
#   LIVE_SCAN          newest matches to scan for a demo (default 10)
#   LIVE_MAX_AGE_DAYS  how old the discovered target may be (default 30)
#   LIVE_KEEP=1        keep the downloaded archive (440-890 MB)
#   EVIDENCE           evidence folder root (default: <repo>/.live-checks)
#   CHROME             Chrome binary (default: the macOS install)
# Exit status: 0 every check passed, 1 a check failed, 2 a prerequisite is missing (a coverage gap).

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v bun >/dev/null; then
  echo "GAP   bun is not installed"
  echo "exit 2: a prerequisite is missing, so this is a coverage gap, not a pass"
  exit 2
fi

exec bun run "$REPO/scripts/live-check.ts" "$@"
