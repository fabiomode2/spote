#!/usr/bin/env python3
"""Lanza un programa de terminal en un pty, interpreta su salida ANSI y guarda
la pantalla como HTML truecolor (o texto).

Sin tmux, sin X, sin emulador externo: un intérprete mínimo de las secuencias
que emite crossterm/ratatui (CUP, SGR, EL, ED, scroll, UTF-8 de ancho variable).

Uso:
  scripts/frame.py --cols 200 --rows 56 --out frame.html \
      --actions "wait 3; key space; wait 2; key Right 8; wait 4" \
      -- env SPOTE_HOME=/tmp/spote-home ./target/debug/spote
"""
from __future__ import annotations

import argparse
import codecs
import fcntl
import html
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
import unicodedata

DEFAULT_FG = (232, 230, 240)
DEFAULT_BG = (11, 11, 16)


def cell_width(ch: str) -> int:
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "dim", "italic", "underline", "wide")

    def __init__(self):
        self.ch = " "
        self.fg = None
        self.bg = None
        self.bold = False
        self.dim = False
        self.italic = False
        self.underline = False
        self.wide = False

    def clone_style(self, other: "Cell"):
        self.fg = other.fg
        self.bg = other.bg
        self.bold = other.bold
        self.dim = other.dim
        self.italic = other.italic
        self.underline = other.underline


def color256(index: int) -> tuple[int, int, int]:
    if index < 16:
        table = [
            (0, 0, 0), (128, 0, 0), (0, 128, 0), (128, 128, 0),
            (0, 0, 128), (128, 0, 128), (0, 128, 128), (192, 192, 192),
            (128, 128, 128), (255, 0, 0), (0, 255, 0), (255, 255, 0),
            (0, 0, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
        ]
        return table[index]
    if index < 232:
        index -= 16
        conv = lambda v: 0 if v == 0 else 55 + v * 40
        return (conv(index // 36), conv((index % 36) // 6), conv(index % 6))
    level = 8 + (index - 232) * 10
    return (level, level, level)


class Screen:
    def __init__(self, cols: int, rows: int):
        self.cols = cols
        self.rows = rows
        self.grid = [[Cell() for _ in range(cols)] for _ in range(rows)]
        self.x = 0
        self.y = 0
        self.fg = None
        self.bg = None
        self.bold = False
        self.dim = False
        self.italic = False
        self.underline = False
        self.scroll_top = 0
        self.scroll_bottom = rows - 1
        self.saved = (0, 0)
        self.last = " "
        self.wrap_pending = False
        self.responses: list[bytes] = []
        self.pending = ""

    # ------------------------------------------------------------- utilidades
    def blank(self, cell: Cell):
        cell.ch = " "
        cell.fg = self.fg
        cell.bg = self.bg
        cell.bold = self.bold
        cell.dim = self.dim
        cell.italic = self.italic
        cell.underline = self.underline
        cell.wide = False

    def clear_all(self):
        for row in self.grid:
            for cell in row:
                self.blank(cell)

    def erase_line(self, mode: int):
        row = self.grid[self.y]
        if mode == 0:
            span = range(self.x, self.cols)
        elif mode == 1:
            span = range(0, self.x + 1)
        else:
            span = range(0, self.cols)
        for i in span:
            self.blank(row[i])

    def erase_display(self, mode: int):
        if mode in (2, 3):
            self.clear_all()
            if mode == 2:
                self.x = self.y = 0
            return
        if mode == 0:
            self.erase_line(0)
            for y in range(self.y + 1, self.rows):
                for cell in self.grid[y]:
                    self.blank(cell)
        elif mode == 1:
            self.erase_line(1)
            for y in range(0, self.y):
                for cell in self.grid[y]:
                    self.blank(cell)

    def scroll_up(self, count: int = 1):
        for _ in range(max(1, count)):
            del self.grid[self.scroll_top]
            row = [Cell() for _ in range(self.cols)]
            for cell in row:
                self.blank(cell)
            self.grid.insert(self.scroll_bottom, row)

    def put(self, ch: str):
        width = cell_width(ch)
        if width == 0:
            if self.x > 0:
                prev = self.grid[self.y][self.x - 1]
                prev.ch += ch
            return
        if self.wrap_pending:
            self.x = 0
            self.linefeed()
            self.wrap_pending = False
        if self.x + width > self.cols:
            self.x = 0
            self.linefeed()
        cell = self.grid[self.y][self.x]
        cell.ch = ch
        cell.fg = self.fg
        cell.bg = self.bg
        cell.bold = self.bold
        cell.dim = self.dim
        cell.italic = self.italic
        cell.underline = self.underline
        cell.wide = width == 2
        if width == 2 and self.x + 1 < self.cols:
            filler = self.grid[self.y][self.x + 1]
            filler.ch = ""
            filler.wide = True
            filler.clone_style(cell)
        self.x += width
        if self.x >= self.cols:
            self.x = self.cols - 1 if self.cols else 0
            self.wrap_pending = True

    def linefeed(self):
        if self.y == self.scroll_bottom:
            self.scroll_up(1)
        else:
            self.y = min(self.rows - 1, self.y + 1)

    def sgr(self, params: list[int]):
        i = 0
        while i < len(params):
            p = params[i]
            if p == 0:
                self.fg = self.bg = None
                self.bold = self.dim = self.italic = self.underline = False
            elif p == 1:
                self.bold = True
            elif p == 2:
                self.dim = True
            elif p == 3:
                self.italic = True
            elif p == 4:
                self.underline = True
            elif p == 7:
                self.fg, self.bg = self.bg, self.fg
            elif p == 22:
                self.bold = self.dim = False
            elif p == 23:
                self.italic = False
            elif p == 24:
                self.underline = False
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
                if i + 1 < len(params) and params[i + 1] == 2 and i + 4 < len(params):
                    value = (params[i + 2], params[i + 3], params[i + 4])
                    if p == 38:
                        self.fg = value
                    else:
                        self.bg = value
                    i += 4
                elif i + 1 < len(params) and params[i + 1] == 5 and i + 2 < len(params):
                    value = color256(params[i + 2])
                    if p == 38:
                        self.fg = value
                    else:
                        self.bg = value
                    i += 2
            i += 1

    # --------------------------------------------------------------- parsing
    def feed(self, text: str):
        if self.pending:
            text = self.pending + text
            self.pending = ""
        i = 0
        n = len(text)
        while i < n:
            ch = text[i]
            if ch == "\x1b":
                consumed = self._escape(text, i)
                if consumed == 0:
                    # Secuencia partida entre dos lecturas del pty.
                    self.pending = text[i:]
                    return
                i += consumed
                continue
            if ch == "\r":
                self.x = 0
                self.wrap_pending = False
            elif ch == "\n":
                self.linefeed()
                self.wrap_pending = False
            elif ch == "\b":
                self.x = max(0, self.x - 1)
                self.wrap_pending = False
            elif ch == "\t":
                self.x = min(self.cols - 1, (self.x // 8 + 1) * 8)
            elif ch == "\x07":
                pass
            elif ord(ch) >= 32:
                self.put(ch)
            i += 1

    def _escape(self, text: str, start: int) -> int:
        # Devuelve bytes consumidos o 0 si la secuencia está incompleta.
        if start + 1 >= len(text):
            return 0
        kind = text[start + 1]
        if kind == "[":
            j = start + 2
            while j < len(text) and not ("@" <= text[j] <= "~"):
                j += 1
            if j >= len(text):
                return 0
            body = text[start + 2 : j]
            final = text[j]
            self._csi(body, final)
            return j - start + 1
        if kind == "]":
            # OSC ... BEL or ST
            end = text.find("\x07", start)
            st = text.find("\x1b\\", start)
            if end == -1 and st == -1:
                return 0
            if st != -1 and (end == -1 or st < end):
                return st - start + 2
            return end - start + 1
        if kind in "()*+":
            return 3 if start + 2 < len(text) else 0
        return 2

    def _csi(self, body: str, final: str):
        private = body.startswith("?") or body.startswith(">") or body.startswith("<")
        clean = body.lstrip("?><=")
        # Consultas del terminal: hay que contestar o crossterm/kitty se cuelgan.
        if final == "n" and clean in ("6", ""):
            self.responses.append(f"\x1b[{self.y + 1};{self.x + 1}R".encode())
            return
        if final == "c" and not private:
            self.responses.append(b"\x1b[?1;2c")
            return
        if final == "u" and body.startswith("?"):
            # Kitty keyboard protocol: no soportado.
            self.responses.append(b"\x1b[?0u")
            return
        parts = [int(p) if p.isdigit() else 0 for p in clean.split(";")] if clean else []
        first = parts[0] if parts else 0
        if private:
            if final == "h" and first == 1049:
                self.clear_all()
                self.x = self.y = 0
            return
        if final in "Hf":
            row = (parts[0] if len(parts) > 0 and parts[0] else 1) - 1
            col = (parts[1] if len(parts) > 1 and parts[1] else 1) - 1
            self.y = max(0, min(self.rows - 1, row))
            self.x = max(0, min(self.cols - 1, col))
            self.wrap_pending = False
        elif final == "A":
            self.y = max(0, self.y - max(1, first))
        elif final == "B":
            self.y = min(self.rows - 1, self.y + max(1, first))
        elif final == "C":
            self.x = min(self.cols - 1, self.x + max(1, first))
        elif final == "D":
            self.x = max(0, self.x - max(1, first))
        elif final == "E":
            self.x = 0
            self.y = min(self.rows - 1, self.y + max(1, first))
        elif final == "F":
            self.x = 0
            self.y = max(0, self.y - max(1, first))
        elif final == "G":
            self.x = max(0, min(self.cols - 1, max(1, first) - 1))
        elif final == "d":
            self.y = max(0, min(self.rows - 1, max(1, first) - 1))
        elif final == "J":
            self.erase_display(first)
        elif final == "K":
            self.erase_line(first)
        elif final == "m":
            self.sgr(parts)
        elif final == "r":
            top = (parts[0] if len(parts) > 0 and parts[0] else 1) - 1
            bottom = (parts[1] if len(parts) > 1 and parts[1] else self.rows) - 1
            self.scroll_top = max(0, min(self.rows - 1, top))
            self.scroll_bottom = max(self.scroll_top, min(self.rows - 1, bottom))
            self.x = self.y = 0
        elif final == "S":
            self.scroll_up(max(1, first))
        elif final == "T":
            for _ in range(max(1, first)):
                del self.grid[self.scroll_bottom]
                row = [Cell() for _ in range(self.cols)]
                for cell in row:
                    self.blank(cell)
                self.grid.insert(self.scroll_top, row)
        elif final == "s":
            self.saved = (self.x, self.y)
        elif final == "u":
            self.x, self.y = self.saved
        elif final == "L":
            for _ in range(max(1, first)):
                row = [Cell() for _ in range(self.cols)]
                for cell in row:
                    self.blank(cell)
                self.grid.insert(self.y, row)
                del self.grid[self.rows]
        elif final == "M":
            for _ in range(max(1, first)):
                del self.grid[self.y]
                row = [Cell() for _ in range(self.cols)]
                for cell in row:
                    self.blank(cell)
                self.grid.append(row)
        elif final == "@":
            row = self.grid[self.y]
            for _ in range(max(1, first)):
                row.insert(self.x, Cell())
                row.pop()
        elif final == "P":
            row = self.grid[self.y]
            for _ in range(max(1, first)):
                if self.x < len(row):
                    del row[self.x]
                    row.append(Cell())
        elif final == "X":
            for i in range(self.x, min(self.cols, self.x + max(1, first))):
                self.blank(self.grid[self.y][i])

    # ------------------------------------------------------------- snapshots
    def to_text(self) -> str:
        lines = []
        for row in self.grid:
            text = "".join(cell.ch if cell.ch else "" for cell in row)
            lines.append(text.rstrip())
        return "\n".join(lines)

    def to_html(self) -> str:
        out = []
        for row in self.grid:
            spans = []
            run_style = None
            run = []

            def flush():
                if not run:
                    return
                fg, bg, bold, dim, italic, underline = run_style
                css = [f"color:{fg}", f"background:{bg}"]
                if bold:
                    css.append("font-weight:700")
                if dim:
                    css.append("opacity:.72")
                if italic:
                    css.append("font-style:italic")
                if underline:
                    css.append("text-decoration:underline")
                spans.append(
                    f'<span style="{";".join(css)}">{html.escape("".join(run))}</span>'
                )

            for cell in row:
                if cell.wide and not cell.ch:
                    continue
                fg = "#%02x%02x%02x" % (cell.fg or DEFAULT_FG)
                bg = "#%02x%02x%02x" % (cell.bg or DEFAULT_BG)
                style = (fg, bg, cell.bold, cell.dim, cell.italic, cell.underline)
                if style != run_style:
                    flush()
                    run = []
                    run_style = style
                run.append(cell.ch or " ")
            flush()
            out.append('<div class="line">' + ("".join(spans) or " ") + "</div>")
        body = "\n".join(out)
        return f"""<!doctype html>
<html><head><meta charset="utf-8"><title>spote frame</title>
<style>
  html,body {{ margin:0;padding:0;background:{'#%02x%02x%02x' % DEFAULT_BG}; }}
  body {{ font-family:'DejaVu Sans Mono','JetBrains Mono',monospace;
          font-size:16px; line-height:19.2px; font-variant-ligatures:none;
          -webkit-font-smoothing:antialiased; padding:12px; }}
  .line {{ white-space:pre; height:19.2px; }}
</style></head><body>
{body}
</body></html>"""


def set_size(fd: int, cols: int, rows: int):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cols", type=int, default=200)
    parser.add_argument("--rows", type=int, default=56)
    parser.add_argument("--out", required=True)
    parser.add_argument("--text-out")
    parser.add_argument("--actions", default="wait 4")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command and args.command[0] == "--" else args.command

    master, slave = pty.openpty()
    set_size(master, args.cols, args.rows)
    env = dict(os.environ)
    env.setdefault("TERM", "xterm-256color")
    env["COLORTERM"] = "truecolor"
    proc = subprocess.Popen(
        command,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        cwd=os.getcwd(),
        preexec_fn=os.setsid,
    )
    os.close(slave)
    screen = Screen(args.cols, args.rows)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    running = True

    def pump(timeout: float):
        nonlocal running
        ready, _, _ = select.select([master], [], [], timeout)
        if not ready:
            return
        try:
            data = os.read(master, 65536)
        except OSError:
            running = False
            return
        if not data:
            running = False
            return
        text = decoder.decode(data)
        raw_out = os.environ.get("SPOTE_RAW_OUT")
        if raw_out:
            with open(raw_out, "ab") as fh:
                fh.write(data)
        screen.feed(text)
        for response in screen.responses:
            try:
                os.write(master, response)
            except OSError:
                running = False
        screen.responses.clear()

    snapshots = 0
    for action in [a.strip() for a in args.actions.split(";") if a.strip()]:
        parts = action.split()
        verb = parts[0]
        if verb == "wait":
            deadline = time.time() + float(parts[1])
            while time.time() < deadline and running:
                pump(0.02)
        elif verb == "key":
            name = parts[1]
            count = int(parts[2]) if len(parts) > 2 else 1
            mapping = {
                "space": b" ",
                "enter": b"\r",
                "tab": b"\t",
                "esc": b"\x1b",
                "up": b"\x1b[A",
                "down": b"\x1b[B",
                "right": b"\x1b[C",
                "left": b"\x1b[D",
                "backspace": b"\x7f",
                "delete": b"\x1b[3~",
                "home": b"\x1b[H",
                "end": b"\x1b[F",
            }
            payload = mapping.get(name.lower(), name.encode())
            for _ in range(count):
                os.write(master, payload)
                deadline = time.time() + 0.12
                while time.time() < deadline:
                    pump(0.02)
        elif verb == "type":
            os.write(master, parts[1].encode())
            deadline = time.time() + 0.2
            while time.time() < deadline:
                pump(0.02)
        elif verb == "resize":
            set_size(master, int(parts[1]), int(parts[2]))
            screen.cols = int(parts[1])
            screen.rows = int(parts[2])
            screen.grid = [[Cell() for _ in range(screen.cols)] for _ in range(screen.rows)]
            screen.scroll_bottom = screen.rows - 1
            os.killpg(os.getpgid(proc.pid), signal.SIGWINCH)
            deadline = time.time() + 0.4
            while time.time() < deadline:
                pump(0.02)
        elif verb == "snap":
            snapshots += 1
            path = parts[1] if len(parts) > 1 else args.out
            open(path, "w", encoding="utf-8").write(screen.to_html())
            print(f"snapshot {path}")
        else:
            print(f"acción desconocida: {action}", file=sys.stderr)

    # Vacía lo que quede pendiente.
    deadline = time.time() + 0.3
    while time.time() < deadline and running:
        pump(0.03)

    open(args.out, "w", encoding="utf-8").write(screen.to_html())
    if args.text_out:
        open(args.text_out, "w", encoding="utf-8").write(screen.to_text())
    print(screen.to_text())
    try:
        os.write(master, b"q")
        time.sleep(0.2)
    except OSError:
        pass
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except ProcessLookupError:
        pass
    proc.wait(timeout=3)
    os.close(master)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
