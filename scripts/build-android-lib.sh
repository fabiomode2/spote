#!/usr/bin/env bash
# Compila el core Rust para Android, genera las bindings Kotlin y deja el .so
# en jniLibs. Pensado para CI y para builds locales completos.
#
#   ./scripts/build-android-lib.sh                 # release arm64 (por defecto)
#   SPOTE_ANDROID_ABIS="arm64-v8a x86_64" ./scripts/build-android-lib.sh
#   ./scripts/build-android-lib.sh android-dev    # iteración rápida
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-android-release}"
ABIS="${SPOTE_ANDROID_ABIS:-arm64-v8a}"
JNI_DIR="$ROOT/android/app/src/main/jniLibs"
BINDINGS_DIR="$ROOT/android/app/src/main/java"

if [[ -z "${ANDROID_NDK_HOME:-}" && -n "${ANDROID_HOME:-}" ]]; then
  NDK_DIR="$(find "$ANDROID_HOME/ndk" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | sort -V | tail -1 || true)"
  if [[ -n "$NDK_DIR" ]]; then
    export ANDROID_NDK_HOME="$NDK_DIR"
  fi
fi

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "cargo-ndk no está instalado: cargo install cargo-ndk --version 4.1.2 --locked" >&2
  exit 1
fi

rustup target add aarch64-linux-android x86_64-linux-android >/dev/null

# Bindings Kotlin desde el .so de host: el metadata de UniFFI es agnóstico de
# arquitectura, así que no hace falta esperar al build de Android.
echo "==> bindings Kotlin"
cargo build -p spote-android
cargo run -p spote-android --features cli --bin uniffi-bindgen -- \
  generate \
  --library target/debug/libspote_android.so \
  --language kotlin \
  --out-dir "$BINDINGS_DIR" \
  --no-format

echo "==> .so Android ($PROFILE): $ABIS"
read -ra ABI_LIST <<< "$ABIS"
NDK_TARGETS=()
for abi in "${ABI_LIST[@]}"; do
  NDK_TARGETS+=(-t "$abi")
done
cargo ndk "${NDK_TARGETS[@]}" -o "$JNI_DIR" build -p spote-android --profile "$PROFILE"

echo "==> listo"
find "$JNI_DIR" -name 'libspote_android.so' -printf '%p %k KB\n' | sort
