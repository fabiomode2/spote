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

    try:
        config = load_config(create=False)
        service = LyricsService(config.lyrics, Cache(cache_dir() / "cache.sqlite3"))
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
    if audio_path:
        aligned_lines = align_with_audio(lines, audio_path, duration_ms / 1000.0)
        if aligned_lines:
            lines, confidence = aligned_lines
            aligned = True
            print(
                f"[lyricbar] alineado con ASR: {len(lines)} líneas, "
                f"confianza {confidence:.2f}",
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


def align_with_audio(
    lines: list[dict], audio_path: str, duration_s: float
) -> tuple[list[dict], float] | None:
    """Alinea las líneas contra la voz real con faster-whisper.

    Es la parte que convierte una letra de Genius repartida a ojo en karaoke
    real. Si falta el modelo o ffmpeg, se devuelve None y queda el LRC normal.
    """
    from pathlib import Path
    import shutil
    import subprocess

    path = Path(audio_path).expanduser()
    if not path.is_file() or shutil.which("ffmpeg") is None:
        return None
    try:
        import numpy as np
        from lyricbar.asr import available as asr_available, transcribe_words
        from lyricbar.lyrics.forced import align_asr_lines
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
    try:
        words = transcribe_words(pcm, model="small", device="auto")
    except Exception:
        return None
    if not words:
        return None
    source = tuple(
        LyricLine(int(line["start_ms"]), line["text"], line.get("end_ms"))
        for line in lines
        if line.get("text")
    )
    try:
        result = align_asr_lines(source, words, duration_s)
    except Exception:
        return None
    if result is None:
        return None
    refined, confidence = result
    if confidence < 0.30:
        return None
    out = []
    for line in refined:
        out.append(
            {
                "start_ms": max(0, int(line.time_ms)),
                "end_ms": max(0, int(line.end_ms)) if line.end_ms is not None else None,
                "text": line.text,
            }
        )
    return out, confidence


if __name__ == "__main__":
    raise SystemExit(main())
