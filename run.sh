#!/usr/bin/env bash
# Arranca spote en local sin comandos largos.
#
#   ./run.sh                 # TUI
#   ./run.sh --follow-spotify
#   ./run.sh --viz inferno
#   SPOTE_HOME=/tmp/x ./run.sh ingest ~/Music
set -euo pipefail
cd "$(dirname "$0")"
export SPOTE_ROOT="$PWD"

BIN="target/release/spote"
if [[ ! -x "$BIN" ]] || [[ -n "$(find crates scripts -newer "$BIN" \( -name '*.rs' -o -name '*.py' \) -print -quit 2>/dev/null)" ]]; then
  echo "· compilando spote (release)…" >&2
  cargo build --release -p spote-tui >&2
fi

exec "$BIN" "$@"
