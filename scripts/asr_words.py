#!/usr/bin/env python3
"""Transcriptor de ventanas para spote.

No conoce la letra ni alinea nada: recibe una ruta de audio y una lista de
ventanas en segundos, y devuelve las palabras que oye con sus tiempos. La
alineación y las relecturas viven en Rust (`crates/core/src/lyrics`), igual
que el resto del motor de letras de la app.

Protocolo: una petición JSON por línea de stdin, una respuesta JSON por línea
de stdout. El proceso se mantiene vivo entre peticiones para cargar el modelo y
decodificar el audio una sola vez.

    {"audio": "/ruta.opus", "windows": [[0, 0], [205, 251]],
     "model": "small", "device": "auto", "language": null}

    {"ok": true, "duration_s": 251.2,
     "reads": [[[10.1, 10.9, "MRS", 0.13], ...], [...]]}

Requiere `faster-whisper` y `ffmpeg`. Si falta algo, responde `ok: false` y la
app sigue mostrando la letra estimada.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

_RATE = 16_000
_PCM: dict[str, "object"] = {}
_MODEL = None
_MODEL_KEY: tuple[str, str, str] | None = None


def _debug(message: str) -> None:
    if os.environ.get("SPOTE_ASR_DEBUG"):
        print(f"[asr] {message}", file=sys.stderr, flush=True)


def _models_root() -> Path:
    explicit = os.environ.get("SPOTE_ASR_MODELS")
    if explicit:
        return Path(explicit).expanduser()
    candidates = [Path.home() / ".cache" / "spote" / "models"]
    home = os.environ.get("SPOTE_HOME")
    if home:
        candidates.insert(0, Path(home) / "cache" / "models")
    # Reutiliza un modelo ya descargado antes de bajar otra copia de medio giga.
    candidates.append(Path.home() / ".cache" / "lyricbar" / "models")
    for candidate in candidates:
        if candidate.is_dir() and any(candidate.iterdir()):
            return candidate
    return candidates[0]


def _decode(path: str):
    import numpy as np

    cached = _PCM.get(path)
    if cached is not None:
        return cached
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise RuntimeError("falta ffmpeg")
    proc = subprocess.run(
        [
            ffmpeg,
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-i",
            path,
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "-ac",
            "1",
            "-ar",
            str(_RATE),
            "-",
        ],
        capture_output=True,
        timeout=180,
    )
    if proc.returncode != 0 or not proc.stdout:
        raise RuntimeError("ffmpeg no pudo decodificar el audio")
    pcm = np.frombuffer(proc.stdout, dtype="<f4").astype(np.float32)
    _PCM[path] = pcm
    return pcm


def _load_model(name: str, device: str):
    global _MODEL, _MODEL_KEY

    from faster_whisper import WhisperModel

    target = name
    candidate = Path(name).expanduser()
    if candidate.exists():
        target = str(candidate)
    root = _models_root()
    key = (target, device, "auto")
    if _MODEL is not None and _MODEL_KEY == key:
        return _MODEL

    if device == "cpu":
        attempts = [("cpu", "int8")]
    elif device == "cuda":
        attempts = [("cuda", "float16"), ("cuda", "int8_float16")]
    else:
        attempts = [
            ("cuda", "float16"),
            ("cuda", "int8_float16"),
            ("cpu", "int8"),
        ]
    last: Exception | None = None
    for current_device, compute_type in attempts:
        try:
            model = WhisperModel(
                target,
                device=current_device,
                compute_type=compute_type,
                cpu_threads=max(1, os.cpu_count() or 4),
                download_root=str(root),
            )
            _MODEL = model
            _MODEL_KEY = key
            _debug(f"modelo {name} en {current_device}/{compute_type}")
            return model
        except Exception as exc:  # CUDA ausente, OOM, tipo no soportado...
            last = exc
            _debug(f"falló {current_device}/{compute_type}: {exc}")
    raise RuntimeError(f"no se pudo cargar whisper: {last}")


def _transcribe(model, pcm, language):
    segments, _info = model.transcribe(
        pcm,
        language=language,
        word_timestamps=True,
        vad_filter=False,
        beam_size=1,
        best_of=1,
        condition_on_previous_text=False,
    )
    words = []
    for segment in segments:
        for word in segment.words or []:
            text = (word.word or "").strip()
            if not text:
                continue
            words.append(
                [
                    round(float(word.start), 3),
                    round(float(word.end), 3),
                    text,
                    round(float(word.probability), 3),
                ]
            )
    return words


def _handle(request: dict) -> dict:
    audio = str(request.get("audio") or "")
    if not audio or not Path(audio).is_file():
        return {"ok": False, "error": "audio no encontrado"}
    model_name = str(request.get("model") or "small")
    device = str(request.get("device") or "auto")
    language = request.get("language")
    if not language or language == "auto":
        language = None

    pcm = _decode(audio)
    duration = float(len(pcm)) / _RATE
    model = _load_model(model_name, device)

    reads = []
    for window in request.get("windows") or []:
        try:
            start_s, end_s = float(window[0]), float(window[1])
        except (TypeError, ValueError, IndexError):
            reads.append([])
            continue
        # `end <= start` (o 0) significa "hasta el final".
        if end_s <= start_s:
            end_s = duration
        begin = max(0, int(start_s * _RATE))
        finish = min(len(pcm), int(end_s * _RATE))
        if finish - begin < _RATE:
            reads.append([])
            continue
        chunk = pcm[begin:finish]
        origin = begin / _RATE
        try:
            words = _transcribe(model, chunk, language)
        except Exception as exc:
            _debug(f"ventana {start_s:.1f}-{end_s:.1f} falló: {exc}")
            reads.append([])
            continue
        for word in words:
            word[0] = round(word[0] + origin, 3)
            word[1] = round(word[1] + origin, 3)
        readers = words
        reads.append(readers)
    return {"ok": True, "duration_s": round(duration, 3), "reads": reads}


def main() -> int:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            response = _handle(request)
        except Exception as exc:  # nunca matar el proceso por una petición
            response = {"ok": False, "error": str(exc)}
        print(json.dumps(response, ensure_ascii=False), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
