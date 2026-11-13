#!/usr/bin/env python3
"""Convierte una captura de tmux (`capture-pane -e`) en HTML con truecolor.

Uso: ansi2html.py <entrada.txt> <salida.html>
"""
from __future__ import annotations

import html
import re
import sys

DEFAULT_FG = "#e8e6f0"
DEFAULT_BG = "#0b0b10"

SGR = re.compile(r"\x1b\[([0-9;]*)m")
OTHER = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][B0]|\x1b[=>]")


def color256(index: int) -> str:
    if index < 16:
        base = [
            (0, 0, 0), (128, 0, 0), (0, 128, 0), (128, 128, 0),
            (0, 0, 128), (128, 0, 128), (0, 128, 128), (192, 192, 192),
            (128, 128, 128), (255, 0, 0), (0, 255, 0), (255, 255, 0),
            (0, 0, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
        ]
        r, g, b = base[index]
        return f"#{r:02x}{g:02x}{b:02x}"
    if index < 232:
        index -= 16
        r = index // 36
        g = (index % 36) // 6
        b = index % 6
        conv = lambda v: 0 if v == 0 else 55 + v * 40
        return f"#{conv(r):02x}{conv(g):02x}{conv(b):02x}"
    level = 8 + (index - 232) * 10
    return f"#{level:02x}{level:02x}{level:02x}"


class Style:
    def __init__(self) -> None:
        self.fg: str | None = None
        self.bg: str | None = None
        self.bold = False
        self.dim = False
        self.italic = False
        self.underline = False
        self.reverse = False

    def apply(self, params: list[int]) -> None:
        i = 0
        while i < len(params):
            p = params[i]
            if p == 0:
                self.__init__()
            elif p == 1:
                self.bold = True
            elif p == 2:
                self.dim = True
            elif p == 3:
                self.italic = True
            elif p == 4:
                self.underline = True
            elif p == 7:
                self.reverse = True
            elif p == 22:
                self.bold = self.dim = False
            elif p == 23:
                self.italic = False
            elif p == 24:
                self.underline = False
            elif p == 27:
                self.reverse = False
            elif p == 39:
                self.fg = None
            elif p == 49:
                self.bg = None
            elif 30 <= p <= 37:
                self.fg = color256(p - 30)
            elif 90 <= p <= 97:
                self.fg = color256(p - 90 + 8)
            elif 40 <= p <= 47:
                self.bg = color256(p - 40)
            elif 100 <= p <= 107:
                self.bg = color256(p - 100 + 8)
            elif p in (38, 48):
                target = "fg" if p == 38 else "bg"
                if i + 1 < len(params) and params[i + 1] == 2 and i + 4 < len(params):
                    r, g, b = params[i + 2], params[i + 3], params[i + 4]
                    setattr(self, target, f"#{r:02x}{g:02x}{b:02x}")
                    i += 4
                elif i + 1 < len(params) and params[i + 1] == 5 and i + 2 < len(params):
                    setattr(self, target, color256(params[i + 2]))
                    i += 2
            i += 1

    def css(self) -> str:
        fg = self.fg or DEFAULT_FG
        bg = self.bg or DEFAULT_BG
        if self.reverse:
            fg, bg = bg, fg
        parts = [f"color:{fg}", f"background:{bg}"]
        if self.bold:
            parts.append("font-weight:700")
        if self.dim:
            parts.append("opacity:0.72")
        if self.italic:
            parts.append("font-style:italic")
        if self.underline:
            parts.append("text-decoration:underline")
        return ";".join(parts)


def render(source: str) -> str:
    lines: list[str] = []
    style = Style()
    for raw in source.split("\n"):
        raw = OTHER.sub("", raw)
        out: list[str] = []
        pos = 0
        for match in SGR.finditer(raw):
            chunk = raw[pos:match.start()]
            if chunk:
                out.append(f'<span style="{style.css()}">{html.escape(chunk)}</span>')
            params = [int(p) if p else 0 for p in match.group(1).split(";")] if match.group(1) else [0]
            style.apply(params)
            pos = match.end()
        chunk = raw[pos:]
        if chunk:
            out.append(f'<span style="{style.css()}">{html.escape(chunk)}</span>')
        if not out:
            out.append(f'<span style="{style.css()}"> </span>')
        lines.append(f'<div class="line">{"".join(out)}</div>')
    body = "\n".join(lines)
    return f"""<!doctype html>
<html><head><meta charset="utf-8">
<style>
  html, body {{ margin:0; padding:0; background:{DEFAULT_BG}; }}
  body {{ font-family: 'DejaVu Sans Mono', 'JetBrains Mono', monospace;
          font-size:16px; line-height:19.2px; letter-spacing:0;
          font-variant-ligatures:none; -webkit-font-smoothing:antialiased;
          padding:14px; }}
  .line {{ white-space:pre; height:19.2px; }}
</style></head>
<body>
{body}
</body></html>
"""


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    source = open(sys.argv[1], "r", encoding="utf-8", errors="replace").read()
    open(sys.argv[2], "w", encoding="utf-8").write(render(source))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
