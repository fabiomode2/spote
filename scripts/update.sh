#!/usr/bin/env bash
# Atajo para actualizar spote: git pull, compilar e instalar.
#
#   ./scripts/update.sh
#
# La TUI hace lo mismo desde el aviso de versión nueva.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/scripts/install.sh" --update "$@"
