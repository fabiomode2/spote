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

    root = Path(data.get("root") or os.environ.get("LYRICBAR_ROOT") or "/home/fabio/Code/lyricbar")
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

    print(
        json.dumps(
            {
                "ok": True,
                "synced": bool(lyrics.synced),
                "estimated": bool(lyrics.estimated),
                "instrumental": bool(lyrics.instrumental),
                "provider": lyrics.provider or resolution.reason or "lyricbar",
                "lines": lines,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
