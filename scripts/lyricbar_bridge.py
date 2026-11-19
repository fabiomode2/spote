#!/usr/bin/env python3
"""Puente entre spote y el motor de letras de lyricbar.

Lee un JSON por stdin (`{title, artists, album, duration_ms, root}`) y escribe
un JSON por stdout con las líneas ya sincronizadas:

    {"ok": true, "synced": true, "provider": "lrclib", "estimated": false,
     "lines": [{"start_ms": 12300, "end_ms": 18900, "text": "..."}]}

lyricbar se encarga de LRCLIB, Genius, las captions de YouTube, el reparto de
letras planas y su propia caché. Si algo falla, se devuelve `ok: false` y spote
sigue con su proveedor interno.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path


def fail(message: str, **extra) -> int:
    payload = {"ok": False, "error": message}
    payload.update(extra)
    print(json.dumps(payload, ensure_ascii=False))
    return 0


def main() -> int:
    try:
        raw = sys.stdin.read()
        data = json.loads(raw) if raw.strip() else {}
    except json.JSONDecodeError as exc:
        return fail(f"json de entrada: {exc}")

    title = (data.get("title") or "").strip()
    if not title:
        return fail("sin título")

    root_raw = data.get("root") or os.environ.get("LYRICBAR_ROOT") or ""
    if not root_raw:
        return fail("sin ruta de lyricbar ([lyrics] root o LYRICBAR_ROOT)")
    root = Path(root_raw)
    if not (root / "lyricbar").is_dir():
        return fail(f"lyricbar no encontrado en {root}")
    sys.path.insert(0, str(root))

    try:
        from lyricbar.cache import Cache
        from lyricbar.config import cache_dir, load_config
        from lyricbar.lyrics.service import LyricsService
        from lyricbar.models import Track
    except Exception as exc:  # pragma: no cover - depende del entorno
        return fail(f"import lyricbar: {exc}")

    artists = data.get("artists") or []
    if isinstance(artists, str):
        artists = [artists]
    duration_ms = data.get("duration_ms") or 0
    audio_path = (data.get("audio_path") or "").strip()
    force_align = bool(data.get("force_align"))

    try:
        config = load_config(create=False)
        cache = Cache(cache_dir() / "cache.sqlite3")
        service = LyricsService(config.lyrics, cache)
        track = Track(
            title=title,
            artists=tuple(a for a in artists if a),
            album=data.get("album") or None,
            duration_s=(duration_ms / 1000.0) if duration_ms else None,
            source="spote",
        )
        resolution = service.resolve(track)
    except Exception as exc:  # pragma: no cover - red, proveedor, etc.
        return fail(f"resolve: {exc}")

    lyrics = resolution.lyrics
    if lyrics is None or lyrics.empty:
        return fail(resolution.reason or "sin letra", found=False)

    lines = []
    for line in lyrics.lines:
        text = (line.text or "").strip()
        if not text:
            continue
        lines.append(
            {
                "start_ms": int(line.time_ms),
                "end_ms": int(line.end_ms) if line.end_ms is not None else None,
                "text": text,
            }
        )

    aligned = False
    if audio_path and needs_alignment(lyrics, lines, force_align=force_align):
        audio_result = align_with_audio(
            lines,
            audio_path,
            duration_ms / 1000.0,
            cache=cache,
            config=config,
        )
        if audio_result:
            lines, confidence, reads = audio_result
            aligned = True
            print(
                f"[lyricbar] alineado con ASR: {len(lines)} líneas, "
                f"confianza {confidence:.2f}, {reads} lectura(s)",
                file=sys.stderr,
            )

    print(
        json.dumps(
            {
                "ok": True,
                "synced": bool(lyrics.synced and not lyrics.estimated) or aligned,
                "estimated": bool(lyrics.estimated and not aligned),
                "aligned": aligned,
                "instrumental": bool(lyrics.instrumental),
                "provider": lyrics.provider or resolution.reason or "lyricbar",
                "lines": lines,
            },
            ensure_ascii=False,
        )
    )
    return 0


def _instrumental(text: str) -> bool:
    """Marcas `♪` que lyricbar intercala; no se alinean, se descartan."""
    return not text.strip().strip("♪").strip()


def needs_alignment(lyrics, lines: list[dict], *, force_align: bool = False) -> bool:
    """¿Merece la pena re-alinear con ASR una letra que ya trae tiempos?

    Un LRC de LRCLIB suele ser mejor que el ASR y no se toca. Se re-alinea
    cuando la letra es estimada/plana o cuando el LRC trae ráfagas
    imposibles (varias líneas a menos de 400 ms), que es el síntoma de un
    envío roto. ``LYRICBAR_ALIGN_SYNCED=1`` fuerza el pase en cualquier caso.
    """
    if force_align or os.environ.get("LYRICBAR_ALIGN_SYNCED") == "1":
        return True
    if lyrics.estimated or not lyrics.synced:
        return True
    times = [line["start_ms"] for line in lines]
    tight = sum(1 for before, after in zip(times, times[1:]) if 0 <= after - before < 400)
    return tight >= 2


def _asr_cache_key(path: Path, model: str, language: str) -> str:
    import hashlib

    stat = path.stat()
    fingerprint = f"{path.resolve()}|{stat.st_mtime_ns}|{stat.st_size}"
    digest = hashlib.sha1(fingerprint.encode()).hexdigest()[:16]
    return f"asr:v3:{digest}:{model}:{language}"


def _cached_words(cache, key: str):
    if cache is None:
        return None
    value = cache.get(key)
    if isinstance(value, dict):
        value = value.get("words")
    if not isinstance(value, list) or not value:
        return None
    try:
        return [
            (float(item[0]), float(item[1]), str(item[2]), float(item[3]))
            for item in value
        ]
    except (TypeError, ValueError, IndexError):
        return None


def _store_words(cache, key: str, words, ttl_days: float) -> None:
    if cache is None or not words:
        return
    ttl = ttl_days * 86_400 if ttl_days > 0 else None
    cache.set(
        key,
        {
            "words": [
                [round(start, 3), round(end, 3), word, round(probability, 3)]
                for start, end, word, probability in words
            ],
            "version": 3,
        },
        ttl=ttl,
    )


def align_with_audio(
    lines: list[dict],
    audio_path: str,
    duration_s: float,
    *,
    cache=None,
    config=None,
) -> tuple[list[dict], float, int] | None:
    """Alinea las líneas contra la voz real con faster-whisper.

    Lee la canción entera una vez y, si el modelo se deja el final o una
    estrofa, vuelve a leer esa zona con contexto a los lados. De las varias
    lecturas se queda la alineación con mejor confianza y menos líneas
    pegadas. Las palabras se cachean por fichero para no repetir el trabajo.
    """
    from pathlib import Path
    import shutil
    import subprocess

    path = Path(audio_path).expanduser()
    if not path.is_file() or shutil.which("ffmpeg") is None:
        return None
    try:
        import numpy as np
        from lyricbar.asr import available as asr_available, model_size, device_preference
        from lyricbar.lyrics.timing import align_pcm
        from lyricbar.models import LyricLine
    except Exception:
        return None
    if not asr_available():
        return None
    try:
        decoded = subprocess.run(
            [
                "ffmpeg",
                "-nostdin",
                "-hide_banner",
                "-v",
                "error",
                "-i",
                str(path),
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-",
            ],
            capture_output=True,
            timeout=90,
            check=False,
        )
    except Exception:
        return None
    if decoded.returncode != 0 or not decoded.stdout:
        return None
    pcm = np.frombuffer(decoded.stdout, dtype="<i2").astype(np.float32) / 32768.0
    if pcm.size < 16000 * 3:
        return None

    sync_cfg = getattr(config, "sync", None)
    model = model_size(getattr(sync_cfg, "asr_model", "small"))
    device = device_preference(getattr(sync_cfg, "asr_device", "auto"))
    language = (getattr(sync_cfg, "asr_language", "auto") or "auto").strip()
    if language == "auto":
        language = None
    ttl_days = float(getattr(sync_cfg, "cache_ttl_days", 90.0) or 0.0)

    key = _asr_cache_key(path, model, language or "auto")
    words = _cached_words(cache, key)

    source = tuple(
        LyricLine(int(line["start_ms"]), line["text"], line.get("end_ms"))
        for line in lines
        if line.get("text") and not _instrumental(line["text"])
    )
    if len(source) < 2:
        return None
    try:
        result = align_pcm(
            pcm,
            source,
            duration_s,
            model=model,
            device=device,
            language=language,
            words=words,
        )
    except Exception:
        return None
    if result is None:
        return None
    _store_words(cache, key, result.words, ttl_days)

    out = []
    for line in result.lines:
        out.append(
            {
                "start_ms": max(0, int(line.time_ms)),
                "end_ms": max(0, int(line.end_ms)) if line.end_ms is not None else None,
                "text": line.text,
            }
        )
    return out, result.confidence, result.reads


if __name__ == "__main__":
    raise SystemExit(main())
