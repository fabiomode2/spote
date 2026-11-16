#!/usr/bin/env bash
# Captura la TUI en una sesión tmux aislada y la convierte en HTML.
#
#   scripts/shot.sh <out.html> [cols] [rows] [segundos] -- <comando…>
#
# El PNG se saca luego con scripts/html2png.py (busca un navegador headless):
#   scripts/html2png.py out.html out.png
#
# Requiere: tmux, python3. No toca la sesión tmux del usuario.
set -euo pipefail

OUT="${1:?falta la ruta de salida}"; shift
COLS="${1:-200}"; shift || true
ROWS="${1:-56}"; shift || true
WAIT="${1:-6}"; shift || true
if [[ "${1:-}" == "--" ]]; then shift; fi
if [[ $# -eq 0 ]]; then echo "falta el comando" >&2; exit 2; fi

SOCK="spote-shot-$$"
TMP="$(mktemp -d)"
trap 'tmux -L "$SOCK" kill-server 2>/dev/null || true; rm -rf "$TMP"' EXIT

tmux -L "$SOCK" -f /dev/null new-session -d -s shot -x "$COLS" -y "$ROWS"
tmux -L "$SOCK" set-option -t shot default-terminal "tmux-256color" >/dev/null
tmux -L "$SOCK" set-option -as terminal-features ",*:RGB" >/dev/null
tmux -L "$SOCK" set-option -g status off >/dev/null

# Ejecuta el comando y deja la terminal abierta para poder capturarla.
printf 'cd %q && %s\n' "$PWD" "$(printf '%q ' "$@")" > "$TMP/cmd"
tmux -L "$SOCK" send-keys -t shot "TERM=xterm-256color COLORTERM=truecolor bash $TMP/cmd" Enter
sleep "$WAIT"
tmux -L "$SOCK" capture-pane -p -e -J -t shot > "$TMP/frame.ansi"
python3 "$(dirname "$0")/ansi2html.py" "$TMP/frame.ansi" "$OUT"
echo "$OUT"
