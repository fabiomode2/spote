#!/usr/bin/env python3
"""Convierte el HTML truecolor de `frame.py` en PNG con un navegador headless.

    scripts/frame.py --cols 200 --rows 56 --out frame.html --actions "wait 4" -- ./target/release/spote
    scripts/html2png.py frame.html now.png

Busca un chromium/chrome en PATH o en la caché de Playwright. No abre ventanas
ni necesita X/Wayland. Por defecto ajusta las métricas para que 200x56 celdas
llenen 2050x1160, como las capturas de docs/screenshots/.
"""
from __future__ import annotations

import argparse
import glob
import os
import pathlib
import shutil
import subprocess
import sys

PAD = 12  # padding del body en frame.py
# Avance de DejaVu Sans Mono / font-size (unidades/em).
ADVANCE = 0.60205


def find_browser() -> str:
    override = os.environ.get("SPOTE_CHROME")
    if override and pathlib.Path(override).is_file():
        return override
    for name in (
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "chrome-headless-shell",
    ):
        path = shutil.which(name)
        if path:
            return path
    for pattern in (
        os.path.expanduser("~/.cache/ms-playwright/chromium_headless_shell-*/chrome-headless-shell-linux64/chrome-headless-shell"),
        os.path.expanduser("~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome"),
        os.path.expanduser("~/.cache/puppeteer/chrome/*/chrome-linux64/chrome"),
    ):
        hits = sorted(glob.glob(pattern))
        if hits:
            return hits[-1]
    sys.exit("no encuentro un navegador headless; define SPOTE_CHROME=/ruta/al/binario")


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("input", type=pathlib.Path)
    p.add_argument("output", type=pathlib.Path)
    p.add_argument("--cols", type=int, default=200)
    p.add_argument("--rows", type=int, default=56)
    p.add_argument("--width", type=int, default=2050)
    p.add_argument("--height", type=int, default=1160)
    args = p.parse_args()

    font = (args.width - 2 * PAD) / args.cols / ADVANCE
    line = (args.height - 2 * PAD) / args.rows
    if line < font * 1.05:
        sys.exit("la rejilla no cabe: sube --height o baja --rows")

    html = args.input.read_text(encoding="utf-8")
    html = (
        html.replace("font-size:16px", f"font-size:{font:.3f}px")
        .replace("line-height:19.2px", f"line-height:{line:.3f}px")
        .replace("height:19.2px", f"height:{line:.3f}px")
    )
    # Los span inline sólo pintan el fondo en la caja de la fuente y dejan
    # franjas entre filas; con inline-block el fondo cubre la celda entera.
    html = html.replace(
        "</style>",
        f".line > span{{display:inline-block;height:{line:.3f}px;"
        f"line-height:{line:.3f}px;vertical-align:top}}\n</style>",
    )
    tmp = args.output.with_suffix(".html")
    tmp.write_text(html, encoding="utf-8")

    cmd = [
        find_browser(),
        "--headless",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--hide-scrollbars",
        "--force-device-scale-factor=1",
        f"--window-size={args.width},{args.height}",
        f"--screenshot={args.output}",
        tmp.as_uri(),
    ]
    result = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=90)
    tmp.unlink(missing_ok=True)
    if result.returncode != 0:
        print(result.stderr.decode(errors="replace")[-2000:], file=sys.stderr)
        return result.returncode
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
