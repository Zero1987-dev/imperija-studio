#!/usr/bin/env python3
"""Generiše ikonu Imperija Studija u svim veličinama i formatima.

Originalna oznaka: uspravni kadar 9:16 sa play trouglom, u brend gradijentu.
Pokretanje:  python3 tools/napravi-ikonu.py
"""
from __future__ import annotations

import struct
import sys
from pathlib import Path

from PySide6.QtCore import QBuffer, QByteArray, QPointF, QRectF, Qt
from PySide6.QtGui import (QBrush, QColor, QPainter, QPainterPath, QPen,
                           QPixmap, QRadialGradient)
from PySide6.QtWidgets import QApplication

ROOT = Path(__file__).resolve().parent.parent
PRIMARY, SECONDARY, VIOLET = "#ff3bd4", "#7130c3", "#a97bff"


def crtaj(size: int) -> QPixmap:
    pm = QPixmap(size, size)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    u = size / 256.0

    tile = QPainterPath()
    tile.addRoundedRect(QRectF(6 * u, 6 * u, 244 * u, 244 * u), 58 * u, 58 * u)
    p.setClipPath(tile)
    p.fillPath(tile, QColor("#0d0a16"))

    for (cx, cy), r, col, a in [((56, 46), 170, PRIMARY, 240),
                                ((206, 214), 180, SECONDARY, 230),
                                ((214, 44), 120, VIOLET, 150)]:
        g = QRadialGradient(cx * u, cy * u, r * u)
        inner, outer = QColor(col), QColor(col)
        inner.setAlpha(a); outer.setAlpha(0)
        g.setColorAt(0.0, inner); g.setColorAt(1.0, outer)
        p.fillRect(pm.rect(), QBrush(g))

    white = QColor("#ffffff")

    # Kruna - veze program za ime (Imperija) i ne lici ni na jedan editor.
    kruna = QPainterPath()
    tacke = [(58, 182), (44, 84), (92, 124), (128, 62), (164, 124), (212, 84), (198, 182)]
    kruna.moveTo(tacke[0][0] * u, tacke[0][1] * u)
    for x, y in tacke[1:]:
        kruna.lineTo(x * u, y * u)
    kruna.closeSubpath()
    p.setPen(Qt.NoPen)
    p.fillPath(kruna, white)

    # Podloga krune. Na 16px se spoji s krunom u mrlju, pa se tada izostavlja.
    if size >= 32:
        p.setPen(QPen(white, max(1.0, 15 * u), Qt.SolidLine, Qt.FlatCap))
        p.drawLine(int(60 * u), int(206 * u), int(196 * u), int(206 * u))

    p.end()
    return pm


def png(size: int) -> bytes:
    store = QByteArray()
    buf = QBuffer(store)
    buf.open(QBuffer.WriteOnly)
    crtaj(size).save(buf, "PNG")
    buf.close()
    return bytes(store)


def napravi_ico(path: Path, sizes: list[int]) -> None:
    images = [(s, png(s)) for s in sizes]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries = payload = b""
    for s, data in images:
        entries += struct.pack("<BBBBHHII", 0 if s >= 256 else s,
                               0 if s >= 256 else s, 0, 0, 1, 32,
                               len(data), offset)
        payload += data
        offset += len(data)
    path.write_bytes(header + entries + payload)


SVG = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="64" height="64">
  <defs>
    <radialGradient id="a" cx="22%" cy="18%" r="70%">
      <stop offset="0" stop-color="#ff3bd4"/><stop offset="1" stop-color="#ff3bd4" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="b" cx="80%" cy="84%" r="72%">
      <stop offset="0" stop-color="#7130c3"/><stop offset="1" stop-color="#7130c3" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect x="6" y="6" width="244" height="244" rx="58" fill="#0d0a16"/>
  <rect x="6" y="6" width="244" height="244" rx="58" fill="url(#a)"/>
  <rect x="6" y="6" width="244" height="244" rx="58" fill="url(#b)"/>
  <path d="M58 182 L44 84 L92 124 L128 62 L164 124 L212 84 L198 182 Z" fill="#fff"/>
  <rect x="60" y="198" width="136" height="16" fill="#fff"/>
</svg>
'''


def main() -> int:
    QApplication.instance() or QApplication([])

    # Imena fajlova ostaju uzvodna (concat_*). Tako nas posao ostaje samo
    # promjena sadrzaja, a spajanje sa novim verzijama programa ne puca.
    (ROOT / "assets" / "concat_logo_512.png").write_bytes(png(512))
    (ROOT / "assets" / "logo-dark.png").write_bytes(png(512))

    icons = ROOT / "assets" / "icons"
    icons.mkdir(parents=True, exist_ok=True)
    for size in (16, 32, 64, 128, 256, 512):
        (icons / f"concat_logo_{size}.png").write_bytes(png(size))
    napravi_ico(icons / "concat.ico", [16, 24, 32, 48, 64, 128, 256])

    # Oznaka unutar samog interfejsa.
    (ROOT / "src" / "crates" / "concat" / "ui" / "assets" / "concat-logo.png").write_bytes(png(64))

    print("  napravljeno:")
    for f in sorted(ROOT.rglob("concat_logo_*.png")):
        if "target" not in str(f):
            print(f"    {f.relative_to(ROOT)}  ({f.stat().st_size} B)")
    for f in [ROOT / "assets" / "logo-dark.png", icons / "concat.ico",
              ROOT / "src/crates/concat/ui/assets/concat-logo.png"]:
        print(f"    {f.relative_to(ROOT)}  ({f.stat().st_size} B)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
