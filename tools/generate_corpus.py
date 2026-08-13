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
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas
from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer

OUT = Path(os.environ.get("CORPUS_DIR", "corpus"))

# invariant=1 on every canvas: omits CreationDate/ModDate so regenerating the
# corpus is byte-reproducible (deterministic input for benchmarks).

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
    doc = SimpleDocTemplate(str(OUT / "dense_textbook.pdf"), pagesize=A4, invariant=1)
    doc.build(story)


def scanned_pages() -> None:
    c = canvas.Canvas(str(OUT / "scanned_pages.pdf"), pagesize=A4, invariant=1)
    for page in range(30):
        img = Image.new("L", (1240, 1754), 250)  # A4 @ ~150 dpi
        draw = ImageDraw.Draw(img)
        for line in range(60):
            y = 80 + line * 28
            x0 = 120 + random.randint(0, 60)
            w = random.randint(200, 900)
            h = random.randint(8, 14)
            draw.rectangle([x0, y, x0 + w, y + h], fill=random.randint(30, 90))
        # Pass the PIL image via ImageReader, not a filename: drawImage() with a
        # path dedups by *filename*, so 30 pages sharing one path would all
        # embed the first page's image. ImageReader dedups by content, so each
        # page keeps its own image (and no temp PNG is needed).
        c.drawImage(ImageReader(img), 0, 0, width=A4[0], height=A4[1])
        c.showPage()
    c.save()


def scientific_paper() -> None:
    c = canvas.Canvas(str(OUT / "scientific_paper.pdf"), pagesize=A4, invariant=1)
    styles = getSampleStyleSheet()
    W, H = A4
    import math
    import textwrap

    # One paper section per page: heading, body text, footer and vector figures
    # all vary per page, so the 12 pages are mutually distinct. Reproducibility
    # comes from a page-local RNG seeded from the fixed 42 (it never touches the
    # global random stream used by scanned_pages).
    sections = [
        (
            "Abstract",
            "Presentamos un motor de rasterización de PDF vectoriales pensado para "
            "tablets de bajo coste con lápiz, con un presupuesto de 60 fps en scroll "
            "y menos de 150 MB de RAM para documentos de 500 páginas. El resumen "
            "describe las contribuciones, la metodología y los resultados principales: "
            "frame time p95 por debajo de 16,6 ms y un caché de páginas limitado por "
            "bytes que mantiene el consumo de memoria acotado.",
        ),
        (
            "Introduction",
            "Los lectores de PDF para dispositivos móviles suelen priorizar la "
            "fidelidad de renderizado por encima de la fluidez. En este trabajo "
            "argumentamos que la fluidez sostenida es la propiedad que define la "
            "experiencia de lectura con lápiz, y de ella derivamos los requisitos de "
            "arquitectura: render a resolución de pantalla, caché limitada por bytes "
            "y anotaciones vectoriales desacopladas del bitmap de página.",
        ),
        (
            "Related Work",
            "Existen múltiples motores de renderizado PDF con licencias dispares. "
            "Este estudio compara el coste de integrar cada motor detrás de una "
            "interfaz común, midiendo tiempo de apertura, memoria residente y "
            "estabilidad del frame time en scroll. Se concluye que la elección del "
            "motor condiciona la licencia del producto final y el presupuesto de "
            "memoria disponible para el resto de la aplicación.",
        ),
        (
            "Methods: Overview",
            "El prototipo se estructura en un núcleo sin interfaz de usuario que "
            "expone un trait de motor de renderizado, un caché LRU limitado por bytes "
            "y un prefetch de páginas colindantes. Toda la lógica de documento se "
            "mantiene fuera del hilo de UI, que nunca se bloquea renderizando. Esta "
            "separación permite probar el núcleo con benchmarks deterministas y "
            "sustituir el motor sin tocar la capa de presentación.",
        ),
        (
            "Methods: Rasterisation",
            "Cada página se rasteriza a la resolución de la pantalla, nunca a la "
            "resolución máxima del documento. Los píxeles se almacenan en un caché "
            "LRU cuyo límite se expresa en bytes y no en número de páginas, de modo "
            "que documentos grandes no multiplican el consumo de memoria. El texto se "
            "extrae de forma perezosa, solo cuando una herramienta lo solicita.",
        ),
        (
            "Methods: Cache Design",
            "El caché mantiene una cola LRU con presupuesto en bytes y prioridad para "
            "las páginas colindantes a la visible. El prefetch se ejecuta en hilos de "
            "fondo con rayon, y la política de expulsión descarta primero las páginas "
            "más antiguas. Las anotaciones se guardan como trazos vectoriales en "
            "coordenadas de página y se dibujan como capa sobre el bitmap, sin "
            "modificar nunca el píxel cacheado.",
        ),
        (
            "Results: Benchmarks",
            "Las mediciones se ejecutan sobre un documento de 500 páginas con un "
            "harness de criterion. El frame time p95 en scroll se mantiene por debajo "
            "de 16,6 ms y el render de una página completa queda por debajo de 25 ms "
            "en la tablet de referencia. La caché caliente reduce el tiempo de "
            "reenvío de página a menos de 2 ms en el peor caso medido.",
        ),
        (
            "Results: Memory",
            "El resident set size se monitoriza con dumpsys meminfo durante un "
            "recorrido completo de 500 páginas. Con la caché limitada a 64 MB, el "
            "pico de RSS se mantiene por debajo de 150 MB, incluyendo el motor de "
            "renderizado y la UI. La página media rasterizada a resolución de "
            "pantalla ocupa menos de 2 MB, lo que confirma el presupuesto de memoria "
            "del diseño.",
        ),
        (
            "Discussion",
            "Los resultados confirman que un diseño centrado en el presupuesto de "
            "memoria permite mantener la fluidez en hardware modesto. La decisión más "
            "costosa es la licencia del motor, que condiciona la distribución del "
            "producto final. El resto de decisiones de arquitectura se derivan de "
            "mediciones repetibles y quedan documentadas como ADR en el repositorio.",
        ),
        (
            "Limitations",
            "El estudio no cubre todavía la sincronización de anotaciones entre "
            "dispositivos ni el consumo energético durante sesiones largas. Los "
            "benchmarks se ejecutan en un único modelo de tablet y una única versión "
            "del motor. La extracción de texto perezosa se valida con un corpus "
            "sintético, pero no con documentos de producción de gran tamaño.",
        ),
        (
            "Conclusion",
            "Un lector de PDF fluido y ligero para tablets de bajo coste es viable "
            "con un motor vectorial, caché limitada por bytes y render a resolución "
            "de pantalla. Las prioridades del proyecto — fluidez, consumo de memoria "
            "y licencia libre — se mantienen compatibles con las mediciones "
            "presentadas. El siguiente hito es el despliegue sobre Android y la "
            "validación con usuarios reales.",
        ),
        (
            "References",
            "MuPDF SDK, documentación de renderizado vectorial. Criterion, harness de "
            "benchmarks para Rust. ReportLab y Pillow, generación del corpus de "
            "pruebas. Documentación de egui/eframe para la UI de prototipo. ADR-001 a "
            "ADR-004 del repositorio PDFLector, donde se registran las decisiones de "
            "arquitectura y sus mediciones asociadas.",
        ),
    ]

    for page, (section, body) in enumerate(sections, start=1):
        # page-local RNG: same seed -> same pages on every regeneration.
        rng = random.Random(42 + page)
        c.setTitle(f"Paper — page {page}: {section}")
        # header + section heading
        c.setFont("Helvetica-Bold", 16)
        c.drawString(30 * mm, H - 30 * mm, "Efficient Rasterisation of Vector PDFs")
        c.setFont("Helvetica-Bold", 11)
        c.drawString(30 * mm, H - 38 * mm, section)
        # body text: section paragraph + deterministic filler, wrapped to column
        c.setFont("Helvetica", 9)
        filler = (
            f"Este párrafo de relleno pertenece a la sección «{section}» de la página "
            f"{page} y sirve para dar cuerpo al texto de forma determinista, de modo "
            "que cada página del corpus sea distinta de las demás y reproducible "
            "byte a byte. "
        ) * 20
        lines = textwrap.wrap(body + " " + filler, width=110)[:38]
        for i, line in enumerate(lines):
            c.drawString(30 * mm, H - (50 + i * 4.2) * mm, line)
        # footer: page number
        c.setFont("Helvetica", 9)
        c.drawCentredString(W / 2, 12 * mm, f"Page {page} of 12")
        # vector figures, distinct per page: sine wave (randomised amplitude,
        # frequency, phase and colour) + bar chart + scatter, all from page RNG
        c.saveState()
        c.translate(30 * mm, 35 * mm)
        amp = rng.uniform(8.0, 18.0)
        freq = rng.uniform(5.0, 11.0)
        phase = rng.uniform(0.0, math.tau)
        c.setStrokeColorRGB(
            rng.uniform(0.05, 0.5), rng.uniform(0.05, 0.5), rng.uniform(0.5, 0.95)
        )
        c.setLineWidth(1.5)
        p = c.beginPath()
        p.moveTo(0, 0)
        for x in range(0, 251, 2):
            p.lineTo(x, amp * math.sin(x / freq + phase) + 12)
        c.drawPath(p)
        # bar chart, right panel
        c.setFillColorRGB(0.4, 0.4, 0.4)
        for i, val in enumerate(rng.randint(2, 9) for _ in range(6)):
            c.rect(280 + i * 18, 0, 12, val * 4, stroke=1, fill=1)
        # scatter points
        c.setFillColorRGB(0.8, 0.2, 0.2)
        for _ in range(12):
            c.circle(280 + rng.uniform(0, 100), rng.uniform(5, 45), 1.4, stroke=1, fill=1)
        c.restoreState()
        c.showPage()
    c.save()


def large_document() -> None:
    c = canvas.Canvas(str(OUT / "large_document.pdf"), pagesize=A4, invariant=1)
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
