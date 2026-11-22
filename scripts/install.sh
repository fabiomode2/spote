#!/usr/bin/env bash
# Instalación y actualización de spote.
#
#   ./scripts/install.sh                  # compila, instala y prepara ASR
#   ./scripts/install.sh --update         # git pull + recompilar + instalar
#   ./scripts/install.sh --no-asr         # sin faster-whisper
#
# No usa sudo ni toca el sistema: binarios en ~/.local/bin y datos en
# ~/.local/share/spote (o $SPOTE_HOME/data). La TUI detecta ese venv sin
# tener que editar el config a mano.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SPOTE_HOME:-}" ]]; then
  DATA_DIR="${SPOTE_DATA_DIR:-$SPOTE_HOME/data}"
else
  DATA_DIR="${SPOTE_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/spote}"
fi
BIN_DIR="${SPOTE_BIN_DIR:-$HOME/.local/bin}"
VENV="${SPOTE_VENV:-$DATA_DIR/venv}"
ASR=1
UPDATE=0
WITH_MINI=1

usage() {
  cat <<EOF
uso: scripts/install.sh [opciones]

  --update              git pull --ff-only antes de compilar
  --no-asr              no instala faster-whisper
  --no-mini             no compila/instala spote-mini (servidores sin GUI)
  --bin-dir <ruta>      destino de los binarios (defecto: ~/.local/bin)
  --venv <ruta>         venv de Python (defecto: $DATA_DIR/venv)
  -y, --yes             sin preguntas (lo usa el auto-actualizador)
  -h, --help            esta ayuda
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --update) UPDATE=1 ;;
    --no-asr) ASR=0 ;;
    --no-mini) WITH_MINI=0 ;;
    --bin-dir)
      BIN_DIR="${2:?falta la ruta de binarios}"
      shift
      ;;
    --venv)
      VENV="${2:?falta la ruta del venv}"
      shift
      ;;
    -y|--yes) ;;
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
if [[ "$WITH_MINI" == 1 ]]; then
  say "compilando spote-mini (release)"
  if ! (cd "$ROOT" && cargo build --release --locked -p spote-mini); then
    warn "spote-mini no compiló (¿faltan libs gráficas?); instala solo la TUI con --no-mini"
  fi
fi

install -d "$BIN_DIR"
say "instalando binarios en $BIN_DIR"
# Se escribe a un temporal y se renombra: reemplazar el binario en marcha con
# install directo puede fallar con «text file busy».
install -m 755 "$ROOT/target/release/spote" "$BIN_DIR/.spote.new"
mv -f "$BIN_DIR/.spote.new" "$BIN_DIR/spote"
if [[ -x "$ROOT/target/release/spote-mini" ]]; then
  install -m 755 "$ROOT/target/release/spote-mini" "$BIN_DIR/.spote-mini.new"
  mv -f "$BIN_DIR/.spote-mini.new" "$BIN_DIR/spote-mini"
fi
if [[ -x "$ROOT/target/release/spoted" ]]; then
  install -m 755 "$ROOT/target/release/spoted" "$BIN_DIR/.spoted.new"
  mv -f "$BIN_DIR/.spoted.new" "$BIN_DIR/spoted"
fi

install -d "$DATA_DIR/scripts"
install -m 755 "$ROOT/scripts/asr_words.py" "$DATA_DIR/scripts/asr_words.py"
# Restos de la versión con puente a lyricbar.
rm -f "$DATA_DIR/scripts/lyricbar_bridge.py"

if [[ "$ASR" == 1 ]]; then
  command -v python3 >/dev/null || {
    echo "falta python3 para el ASR" >&2
    exit 1
  }
  if [[ ! -x "$VENV/bin/python" ]]; then
    say "creando venv de Python en $VENV"
    python3 -m venv "$VENV"
  fi
  "$VENV/bin/python" -m pip install --quiet --upgrade pip >/dev/null
  if ! "$VENV/bin/python" -c 'import faster_whisper' 2>/dev/null; then
    say "instalando faster-whisper (puede tardar)"
    "$VENV/bin/python" -m pip install --quiet faster-whisper
  fi
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) warn "añade $BIN_DIR al PATH para lanzar «spote» desde cualquier sitio" ;;
esac

say "listo · lanza: spote"
say "letras: LRCLIB + Genius + captions de YouTube"
[[ "$ASR" == 1 ]] && say "timings: ASR local con faster-whisper"
