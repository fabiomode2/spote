#!/usr/bin/env bash
# Instalación y actualización de spote.
#
#   ./scripts/install.sh                  # compila, instala y prepara ASR
#   ./scripts/install.sh --lyricbar ~/Code/lyricbar
#   ./scripts/install.sh --update         # git pull + recompilar + instalar
#   ./scripts/install.sh --no-asr         # sin faster-whisper
#
# No usa sudo ni toca el sistema: binarios en ~/.local/bin y datos en
# ~/.local/share/spote (o $SPOTE_HOME/data). La TUI detecta ese venv y el
# lyricbar enlazado sin tener que editar el config a mano.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SPOTE_HOME:-}" ]]; then
  DATA_DIR="${SPOTE_DATA_DIR:-$SPOTE_HOME/data}"
else
  DATA_DIR="${SPOTE_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/spote}"
fi
BIN_DIR="${SPOTE_BIN_DIR:-$HOME/.local/bin}"
VENV="${SPOTE_VENV:-$DATA_DIR/venv}"
LYRICBAR="${LYRICBAR_ROOT:-}"
ASR=1
UPDATE=0
YES=0

usage() {
  cat <<EOF
uso: scripts/install.sh [opciones]

  --update              git pull --ff-only antes de compilar
  --lyricbar <ruta>     checkout de lyricbar (Genius y captions)
  --no-asr              no instala faster-whisper
  --bin-dir <ruta>      destino de los binarios (defecto: ~/.local/bin)
  --venv <ruta>         venv de Python (defecto: $DATA_DIR/venv)
  -y, --yes             sin preguntas (lo usa el auto-actualizador)
  -h, --help            esta ayuda
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --update) UPDATE=1 ;;
    --lyricbar)
      LYRICBAR="${2:?falta la ruta de lyricbar}"
      shift
      ;;
    --no-asr) ASR=0 ;;
    --bin-dir)
      BIN_DIR="${2:?falta la ruta de binarios}"
      shift
      ;;
    --venv)
      VENV="${2:?falta la ruta del venv}"
      shift
      ;;
    -y|--yes) YES=1 ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "opción desconocida: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

say() { printf '· %s\n' "$*" >&2; }
warn() { printf '! %s\n' "$*" >&2; }

if [[ "$UPDATE" == 1 ]]; then
  if [[ -d "$ROOT/.git" ]]; then
    say "actualizando fuentes"
    if ! git -C "$ROOT" pull --ff-only; then
      warn "git pull falló; ¿tienes cambios locales? se instala lo que hay"
    fi
  else
    warn "esto no es un checkout git; no hay nada que actualizar"
  fi
fi

command -v cargo >/dev/null || {
  echo "falta cargo. Instala Rust: https://rustup.rs" >&2
  exit 1
}
for tool in mpv ffmpeg yt-dlp; do
  command -v "$tool" >/dev/null || warn "falta $tool en el PATH (la app funcionará a medias)"
done

say "compilando spote (release)"
(cd "$ROOT" && cargo build --release --locked -p spote-tui -p spote-server)

install -d "$BIN_DIR"
say "instalando binarios en $BIN_DIR"
# Se escribe a un temporal y se renombra: reemplazar el binario en marcha con
# install directo puede fallar con «text file busy».
install -m 755 "$ROOT/target/release/spote" "$BIN_DIR/.spote.new"
mv -f "$BIN_DIR/.spote.new" "$BIN_DIR/spote"
if [[ -x "$ROOT/target/release/spoted" ]]; then
  install -m 755 "$ROOT/target/release/spoted" "$BIN_DIR/.spoted.new"
  mv -f "$BIN_DIR/.spoted.new" "$BIN_DIR/spoted"
fi

install -d "$DATA_DIR/scripts"
install -m 755 "$ROOT/scripts/asr_words.py" "$DATA_DIR/scripts/asr_words.py"
install -m 755 "$ROOT/scripts/lyricbar_bridge.py" "$DATA_DIR/scripts/lyricbar_bridge.py"

# Sin --lyricbar, intenta el checkout típico junto al repo y el del entorno.
if [[ -z "$LYRICBAR" ]]; then
  for candidate in "$ROOT/../lyricbar" "$HOME/Code/lyricbar"; do
    if [[ -d "$candidate/lyricbar" ]]; then
      LYRICBAR="$candidate"
      break
    fi
  done
fi
if [[ -n "$LYRICBAR" ]]; then
  if [[ ! -d "$LYRICBAR" ]]; then
    warn "$LYRICBAR no existe; se instala sin lyricbar"
    LYRICBAR=""
  else
    LYRICBAR="$(cd "$LYRICBAR" && pwd)"
    if [[ ! -d "$LYRICBAR/lyricbar" ]]; then
      warn "$LYRICBAR no parece un checkout de lyricbar; se ignora"
      LYRICBAR=""
    fi
  fi
fi

if [[ -n "$LYRICBAR" ]]; then
  if [[ -L "$DATA_DIR/lyricbar" || ! -e "$DATA_DIR/lyricbar" ]]; then
    ln -sfn "$LYRICBAR" "$DATA_DIR/lyricbar"
    say "lyricbar enlazado desde $LYRICBAR"
  else
    warn "$DATA_DIR/lyricbar ya existe y no es un enlace; se respeta"
  fi
else
  warn "sin lyricbar: LRCLIB + Genius internos (captions de YouTube con --lyricbar)"
fi

if [[ "$ASR" == 1 || -n "$LYRICBAR" ]]; then
  command -v python3 >/dev/null || {
    echo "falta python3 para el ASR/puente de letras" >&2
    exit 1
  }
  if [[ ! -x "$VENV/bin/python" ]]; then
    say "creando venv de Python en $VENV"
    python3 -m venv "$VENV"
  fi
  "$VENV/bin/python" -m pip install --quiet --upgrade pip >/dev/null
  if [[ "$ASR" == 1 ]] && ! "$VENV/bin/python" -c 'import faster_whisper' 2>/dev/null; then
    say "instalando faster-whisper (puede tardar)"
    "$VENV/bin/python" -m pip install --quiet faster-whisper
  fi
  if [[ -n "$LYRICBAR" ]] && ! "$VENV/bin/python" -c 'import requests, bs4, yt_dlp' 2>/dev/null; then
    say "instalando dependencias de lyricbar"
    "$VENV/bin/python" -m pip install --quiet requests beautifulsoup4 yt-dlp
  fi
  if ! command -v yt-dlp >/dev/null && [[ -x "$VENV/bin/yt-dlp" ]]; then
    ln -sfn "$VENV/bin/yt-dlp" "$BIN_DIR/yt-dlp"
    say "yt-dlp enlazado en $BIN_DIR"
  fi
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) warn "añade $BIN_DIR al PATH para lanzar «spote» desde cualquier sitio" ;;
esac

say "listo · lanza: spote"
if [[ -n "$LYRICBAR" ]]; then
  say "letras: LRCLIB + Genius internos + captions (lyricbar) + ASR"
elif [[ "$ASR" == 1 ]]; then
  say "letras: LRCLIB + Genius internos + ASR (sin captions de YouTube)"
fi
