#!/usr/bin/env python3
"""Generate OCR test fixtures into tests/fixtures/ocr/.

Creates:
  scan_page.png   A4-ratio image with printed Chinese + English text
  scanned.pdf     one-page PDF whose only content is that image
  mixed.pdf       page 1 real text, page 2 the image
  text.pdf        pure text PDF (no OCR needed)
  scan_image.docx DOCX embedding the image between paragraphs

Requires: PIL, reportlab, python-docx, and a Noto CJK font (fc-match).
"""
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
from reportlab.lib.pagesizes import A4
from reportlab.pdfgen import canvas
import docx

OUT = Path(__file__).resolve().parent.parent / "tests" / "fixtures-ocr"
OUT.mkdir(parents=True, exist_ok=True)

LINES = ["智能文档转换系统", "Any2md OCR Integration Test", "订单编号：20260810", "合计金额 1,234.56 元"]


def find_cjk_font() -> str:
    out = subprocess.check_output(["fc-match", "-f", "%{file}", "Noto Sans CJK SC"], text=True)
    if not out or not Path(out).exists():
        raise SystemExit(f"no CJK font found: {out!r}")
    return out


def make_scan_png() -> Path:
    path = OUT / "scan_page.png"
    # 150 DPI A4: 1240x1754 (paper aspect ratio ~0.707).
    img = Image.new("RGB", (1240, 1754), "white")
    draw = ImageDraw.Draw(img)
    font = ImageFont.truetype(find_cjk_font(), 64)
    y = 200
    for line in LINES:
        draw.text((120, y), line, fill="black", font=font)
        y += 160
    img.save(path)
    return path


def make_pdfs(png: Path) -> None:
    w, h = A4  # 595 x 842 pt

    c = canvas.Canvas(str(OUT / "scanned.pdf"), pagesize=A4)
    c.drawImage(str(png), 0, 0, width=w, height=h)
    c.showPage()
    c.save()

    c = canvas.Canvas(str(OUT / "mixed.pdf"), pagesize=A4)
    c.setFont("Helvetica", 16)
    c.drawString(72, h - 72, "This page is real text, no OCR needed.")
    c.showPage()
    c.drawImage(str(png), 0, 0, width=w, height=h)
    c.showPage()
    c.save()

    c = canvas.Canvas(str(OUT / "text.pdf"), pagesize=A4)
    c.setFont("Helvetica", 16)
    c.drawString(72, h - 72, "Hello from a plain text PDF page.")
    c.showPage()
    c.save()


def make_docx(png: Path) -> None:
    doc = docx.Document()
    doc.add_heading("Scan fixture", level=1)
    doc.add_paragraph("The image below is a scanned page and should be OCR'd.")
    doc.add_picture(str(png))  # full-width picture: paper aspect ratio
    doc.save(OUT / "scan_image.docx")


if __name__ == "__main__":
    png = make_scan_png()
    make_pdfs(png)
    make_docx(png)
    print("fixtures written to", OUT)
