#!/usr/bin/env python3
"""Generate the PDFLector test corpus.

Cases (per PLAN.md 2.2):
  1. dense_textbook.pdf  -> libro de texto denso (mucho texto)
  2. scanned_pages.pdf   -> PDF escaneado (páginas a imagen)
  3. scientific_paper.pdf-> paper científico (gráficos vectoriales)
  4. large_document.pdf  -> documento grande de 500+ páginas
"""
from __future__ import annotations

import os
import random
from pathlib import Path

from PIL import Image, ImageDraw
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import getSampleStyleSheet
from reportlab.lib.units import mm
from reportlab.pdfgen import canvas
from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer

OUT = Path(os.environ.get("CORPUS_DIR", "corpus"))

LONG_TEXT = (
    "Los procesos de renderizado de documentos convierten el contenido vectorial "
    "en píxeles a una resolución determinada. La calidad percibida depende de la "
    "resolución de salida y del suavizado aplicado a curvas y texto. "
) * 40  # ~1200 words of filler, deterministic


def seed_random() -> None:
    random.seed(42)


def dense_textbook() -> None:
    styles = getSampleStyleSheet()
    story = []
    for chapter in range(1, 9):
        story.append(Paragraph(f"Capítulo {chapter}: Fundamentos", styles["Heading1"]))
        for _ in range(45):
            story.append(Paragraph(LONG_TEXT[:1200], styles["BodyText"]))
            story.append(Spacer(1, 4 * mm))
    doc = SimpleDocTemplate(str(OUT / "dense_textbook.pdf"), pagesize=A4)
    doc.build(story)


def scanned_pages() -> None:
    c = canvas.Canvas(str(OUT / "scanned_pages.pdf"), pagesize=A4)
    for page in range(30):
        img = Image.new("L", (1240, 1754), 250)  # A4 @ ~150 dpi
        draw = ImageDraw.Draw(img)
        for line in range(60):
            y = 80 + line * 28
            x0 = 120 + random.randint(0, 60)
            w = random.randint(200, 900)
            h = random.randint(8, 14)
            draw.rectangle([x0, y, x0 + w, y + h], fill=random.randint(30, 90))
        img.save("/tmp/opencode/_scan_tmp.png")
        c.drawImage("/tmp/opencode/_scan_tmp.png", 0, 0, width=A4[0], height=A4[1])
        c.showPage()
    c.save()
    os.remove("/tmp/opencode/_scan_tmp.png")


def scientific_paper() -> None:
    c = canvas.Canvas(str(OUT / "scientific_paper.pdf"), pagesize=A4)
    styles = getSampleStyleSheet()
    W, H = A4
    for page in range(1, 13):
        c.setTitle(f"Paper — page {page}")
        c.setFont("Helvetica-Bold", 16)
        c.drawString(30 * mm, H - 30 * mm, "Efficient Rasterisation of Vector PDFs")
        c.setFont("Helvetica", 10)
        c.drawString(30 * mm, H - 38 * mm, "Autor, A.; Investigador, B.")
        c.setFont("Helvetica", 9)
        text = (LONG_TEXT[:2000] + "\n") * 3
        for i, line in enumerate(text.splitlines()[:52]):
            c.drawString(30 * mm, H - (50 + i * 4.2) * mm, line[:110])
        # vector "figures": sine wave + bar chart, drawn as vector paths
        c.setStrokeColorRGB(0.1, 0.1, 0.8)
        c.setLineWidth(1.5)
        c.setDash()
        c.translate(30 * mm, 60 * mm)
        import math

        p = c.beginPath()
        p.moveTo(0, 0)
        for x in range(0, 121):
            p.lineTo(x, 15 * math.sin(x / 9.0) + 8)
        c.drawPath(p)
        c.setFillColorRGB(0.4, 0.4, 0.4)
        for i, val in enumerate([3, 7, 5, 9, 4, 8]):
            c.rect(i * 18, 0, 12, val * 4, stroke=1, fill=1)
        c.showPage()
    c.save()


def large_document() -> None:
    c = canvas.Canvas(str(OUT / "large_document.pdf"), pagesize=A4)
    for page in range(1, 501):
        c.setFont("Helvetica-Bold", 14)
        c.drawString(40, 780, f"Sección {page}")
        c.setFont("Helvetica", 10)
        for i in range(40):
            c.drawString(40, 740 - i * 16, LONG_TEXT[i * 40 : i * 40 + 90])
        c.showPage()
    c.save()


def main() -> None:
    seed_random()
    OUT.mkdir(parents=True, exist_ok=True)
    dense_textbook()
    scanned_pages()
    scientific_paper()
    large_document()
    for f in sorted(OUT.glob("*.pdf")):
        print(f"{f.stat().st_size/1e6:8.2f} MB  {f.name}")


if __name__ == "__main__":
    main()
