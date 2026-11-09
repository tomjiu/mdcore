#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""#634 PDFium 正确显示专项——引擎层显示矩阵夹具生成（十类真实形态）。

产出（本目录）：
  1. 十类 PDF 夹具：chinese_embedded / chinese_missing_font / scanned_gray /
     vector_graphics / transparency_blend / cropped_mediabox / rotation_matrix /
     landscape_mixed / multipage_120 / tiny_note
  2. manifest.json——pypdf 独立几何（跨引擎几何断言基准：MediaBox /
     CropBox / /Rotate / display_size）+ 探针页 + parity 阈值声明

设计原则：
  - 确定性生成（无随机、无时间戳——同脚本重跑内容一致）
  - 几何多样性：真 CropBox 偏移原点 / /Rotate 90·180 / 真异 MediaBox
    横竖混排 / 极窄 aspect——引擎层 geometry_contract 测试面逐类断言
  - 渲染内容可对照：墨迹形态在 pdfium 与 poppler（pdftoppm -cropbox
    -gray）同 DPI 光栅下可做 NCC + 墨区 IoU 对照（parity_vs_poppler 面）
  - 非嵌入 CID 字体（chinese_missing_font）低阈值单列——两引擎字体
    回退的形态漂移是已知差异源（0.80/0.60 实测校准）

管线：reportlab 基础生成 → pypdf 后处理（/Rotate、/CropBox）→
      pypdf 读回独立几何 → manifest.json。
"""
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
from pypdf import PdfReader, PdfWriter
from pypdf.generic import NameObject, NumberObject, RectangleObject
from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

DIR = Path(__file__).resolve().parent
FONT_SC = "/usr/share/fonts/truetype/chinese/NotoSansSC-Regular.ttf"

A4_W, A4_H = A4  # 595.2756 × 841.8898 pt

ZH_TITLE = "多引擎显示一致性验证矩阵"
ZH_BODY_LINES = [
    "第一行：渲染显示域由 CropBox 与页旋转共同决定。",
    "第二行：坐标变换必须包含 y 轴翻转与原点偏移。",
    "第三行：搜索高亮的四角映射走用户空间到屏幕空间。",
    "第四行：批注落盘与位图像素必须共享同一 DPI 基准。",
    "Fifth line: mixed CJK and Latin text in one paragraph.",
]


def _zh(c, x, y, text, size, font="NotoSansSC"):
    c.setFont(font, size)
    c.drawString(x, y, text)


# ---------------------------------------------------------------- 夹具构造

def build_chinese_embedded():
    """嵌入 NotoSansSC 子集——两引擎字形一致（高阈值对照）。"""
    path = DIR / "chinese_embedded.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    for page in (1, 2):
        _zh(c, 72, A4_H - 100, ZH_TITLE, 24)
        c.setFont("Helvetica", 10)
        c.drawString(72, A4_H - 124, f"chinese_embedded.pdf — page {page}/2")
        y = A4_H - 170
        for line in ZH_BODY_LINES:
            _zh(c, 72, y, line, 13)
            y -= 26
        c.setFillColorRGB(0.15, 0.35, 0.75)
        c.rect(72, y - 30, A4_W - 144, 14, stroke=0, fill=1)
        c.showPage()
    c.save()
    return path, {"pages": 2, "probe_pages": [1, 2]}


def build_chinese_missing_font():
    """非嵌入 CID（STSong-Light）——两引擎字体回退漂移（低阈值单列）。"""
    pdfmetrics.registerFont(UnicodeCIDFont("STSong-Light"))
    path = DIR / "chinese_missing_font.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    for page in (1, 2):
        _zh(c, 72, A4_H - 100, ZH_TITLE, 24, font="STSong-Light")
        c.setFont("Helvetica", 10)
        c.drawString(72, A4_H - 124, f"chinese_missing_font.pdf — page {page}/2")
        y = A4_H - 170
        for line in ZH_BODY_LINES[:3]:
            _zh(c, 72, y, line, 13, font="STSong-Light")
            y -= 26
        c.setFillColorRGB(0.6, 0.2, 0.2)
        c.rect(72, y - 30, A4_W - 144, 14, stroke=0, fill=1)
        c.showPage()
    c.save()
    return path, {"pages": 2, "probe_pages": [1, 2]}


def build_scanned_gray():
    """整页灰度 JPEG（扫描件语义——零文本层，DCT 解码对照）。"""
    img = Image.new("L", (1240, 1754), 245)  # 150 DPI A4 等效灰度
    d = ImageDraw.Draw(img)
    title = ImageFont.truetype(FONT_SC, 64)
    body = ImageFont.truetype(FONT_SC, 40)
    d.text((90, 110), ZH_TITLE, fill=25, font=title)
    y = 230
    for line in ZH_BODY_LINES:
        d.text((90, y), line, fill=40, font=body)
        y += 74
    d.rectangle((90, y + 20, 1150, y + 60), fill=70)
    d.line((90, 1560, 1150, 1560), fill=90, width=3)
    jpg = DIR / "_scanned_gray.jpg"
    img.save(jpg, "JPEG", quality=82)
    path = DIR / "scanned_gray.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    c.drawImage(str(jpg), 0, 0, A4_W, A4_H)
    c.showPage()
    c.save()
    jpg.unlink()
    return path, {"pages": 1, "probe_pages": [1]}


def build_vector_graphics():
    """纯矢量（线/矩形/圆/贝塞尔）——抗锯齿形态对照。"""
    path = DIR / "vector_graphics.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    c.setLineWidth(2)
    # 网格线
    c.setStrokeColorRGB(0.55, 0.55, 0.55)
    for i in range(1, 9):
        x = A4_W * i / 9
        c.line(x, 60, x, A4_H - 60)
    for i in range(1, 13):
        y = 60 + (A4_H - 120) * i / 13
        c.line(40, y, A4_W - 40, y)
    # 实心矩形（不同灰度）
    for i, g in enumerate((0.1, 0.3, 0.5, 0.7)):
        c.setFillGray(g)
        c.rect(70 + i * 120, A4_H - 220, 90, 70, stroke=0, fill=1)
    # 圆 + 弧
    c.setFillColorRGB(0.8, 0.2, 0.2)
    c.circle(A4_W / 2, A4_H / 2, 90, stroke=1, fill=1)
    c.setStrokeColorRGB(0.1, 0.1, 0.6)
    c.setLineWidth(6)
    c.arc(120, 150, 480, 400, 30, 240)
    # 贝塞尔
    p = c.beginPath()
    p.moveTo(90, 160)
    p.curveTo(200, 60, 380, 300, 520, 150)
    c.setLineWidth(4)
    c.setStrokeColorRGB(0, 0.45, 0.2)
    c.drawPath(p, stroke=1, fill=0)
    c.setFont("Helvetica", 11)
    c.drawString(72, 46, "vector_graphics.pdf — vector only, no image")
    c.showPage()
    c.save()
    return path, {"pages": 1, "probe_pages": [1]}


def build_transparency_blend():
    """ExtGState alpha 混合——半透明叠加形态对照。"""
    path = DIR / "transparency_blend.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    c.setFillColorRGB(1.0, 1.0, 0.85)
    c.rect(0, 0, A4_W, A4_H, stroke=0, fill=1)
    c.setFillColorRGB(0.9, 0.25, 0.2)
    c.setFillAlpha(0.45)
    c.rect(80, A4_H - 420, 300, 260, stroke=0, fill=1)
    c.setFillAlpha(0.6)
    c.setFillColorRGB(0.2, 0.35, 0.85)
    c.circle(320, A4_H - 300, 130, stroke=0, fill=1)
    c.setFillAlpha(0.3)
    c.setFillColorRGB(0.15, 0.6, 0.25)
    c.rect(210, 190, 340, 240, stroke=0, fill=1)
    c.setFillAlpha(0.8)
    c.setFillGray(0.1)
    c.rect(150, 120, 130, 70, stroke=0, fill=1)
    c.setFillAlpha(1.0)
    c.setFont("Helvetica", 11)
    c.drawString(72, 60, "transparency_blend.pdf — alpha 0.3/0.45/0.6/0.8")
    c.showPage()
    c.save()
    return path, {"pages": 1, "probe_pages": [1]}


def build_cropped_mediabox():
    """MediaBox 842×596 全域绘制 + CropBox [101,51,741,545]（偏移原点 +
    内缩）——裁剪生效与「显示域 ≠ MediaBox」验证。"""
    W, H = 842.0, 596.0
    path = DIR / "_cropped_base.pdf"
    c = canvas.Canvas(str(path), pagesize=(W, H))
    # 四角标记（部分将被裁掉——裁剪生效的像素证据）
    c.setFillGray(0.05)
    for (x, y) in [(20, 20), (W - 140, 20), (20, H - 90), (W - 140, H - 90)]:
        c.rect(x, y, 120, 70, stroke=0, fill=1)
    c.setFillColorRGB(0.15, 0.35, 0.75)
    c.rect(220, 200, 400, 200, stroke=0, fill=1)
    c.setFillGray(0.25)
    c.circle(620, 300, 80, stroke=0, fill=1)
    c.setFont("Helvetica", 12)
    c.drawString(240, 250, "CROP ZONE — visible after crop")
    c.showPage()
    c.save()
    out = DIR / "cropped_mediabox.pdf"
    _postprocess(path, out, crop=[101.0, 51.0, 741.0, 545.0])
    path.unlink()
    return out, {"pages": 1, "probe_pages": [1]}


def build_rotation_matrix():
    """/Rotate 90（页 1）与 /Rotate 180（页 2）——方向特征：用户空间顶部
    粗黑条 + 底部细灰条 + 左缘竖条（渲染后位置随旋转迁移，可像素断言）。"""
    path = DIR / "_rotation_base.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    for page in (1, 2):
        c.setFillGray(0.03)  # 顶部粗黑条（用户空间）
        c.rect(0, A4_H - 26, A4_W, 26, stroke=0, fill=1)
        c.setFillGray(0.55)  # 底部细灰条
        c.rect(0, 0, A4_W, 12, stroke=0, fill=1)
        c.setFillGray(0.03)  # 左缘竖条
        c.rect(0, 0, 14, A4_H, stroke=0, fill=1)
        c.setFillColorRGB(0.7, 0.15, 0.15)
        c.circle(A4_W / 2, A4_H / 2, 120, stroke=0, fill=1)
        c.setFillGray(0.0)
        c.setFont("Helvetica-Bold", 72)
        c.drawCentredString(A4_W / 2, A4_H / 2 - 24, f"R{90 if page == 1 else 180}")
        c.showPage()
    c.save()
    out = DIR / "rotation_matrix.pdf"
    _postprocess(path, out, rotates=[90, 180])
    path.unlink()
    return out, {"pages": 2, "probe_pages": [1, 2]}


def build_landscape_mixed():
    """3 页真异 MediaBox：纵 A4 / 横 A4 / 方形——页尺寸混排（等高槽
    letterbox 场景的引擎面几何基准）。"""
    path = DIR / "landscape_mixed.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    sizes = [("portrait A4", A4_W, A4_H), ("landscape A4", A4_H, A4_W), ("square 600", 600.0, 600.0)]
    for i, (label, w, h) in enumerate(sizes, 1):
        c.setPageSize((w, h))
        c.setLineWidth(3)
        c.setStrokeColorRGB(0.1 + 0.25 * i, 0.3, 0.6)
        c.rect(30, 30, w - 60, h - 60, stroke=1, fill=0)
        c.setFillGray(0.08)
        c.rect(w / 2 - 110, h / 2 - 40, 220, 80, stroke=0, fill=1)
        c.setFillColorRGB(1, 1, 1)
        c.setFont("Helvetica-Bold", 26)
        c.drawCentredString(w / 2, h / 2 - 10, f"{i} {label}")
        c.showPage()
    c.save()
    return path, {"pages": 3, "probe_pages": [1, 2, 3]}


def build_multipage_120():
    """120 页——多页确定性与跨文档面（页号条长度随页变化）。"""
    path = DIR / "multipage_120.pdf"
    c = canvas.Canvas(str(path), pagesize=A4)
    for page in range(1, 121):
        c.setFillGray(0.1)
        c.setFont("Helvetica-Bold", 40)
        c.drawRightString(A4_W - 60, A4_H - 100, f"{page:03d}")
        # 页号比例条（长度 = 页号占比——页间内容互异）
        c.setFillColorRGB(0.2, 0.4, 0.75)
        c.rect(60, A4_H - 150, (A4_W - 120) * page / 120.0, 18, stroke=0, fill=1)
        c.setFillGray(0.85)
        c.rect(60, 80, A4_W - 120, 4 + (page % 7), stroke=0, fill=1)
        c.showPage()
    c.save()
    return path, {"pages": 120, "probe_pages": [1, 60, 120]}


def build_tiny_note():
    """极窄页 40×720（aspect 18）——极端纵横比渲染与逐页槽几何。"""
    W, H = 40.0, 720.0
    path = DIR / "tiny_note.pdf"
    c = canvas.Canvas(str(path), pagesize=(W, H))
    c.setFillGray(0.05)
    c.rect(0, 0, 6, H, stroke=0, fill=1)  # 左缘全高窄条
    for i in range(12):
        y = H - 40 - i * 56
        c.setFillGray(0.1 + 0.06 * (i % 5))
        c.rect(12, y, 22, 30, stroke=0, fill=1)
    c.showPage()
    c.save()
    return path, {"pages": 1, "probe_pages": [1]}


# ---------------------------------------------------------------- 后处理

def _postprocess(src: Path, dst: Path, crop=None, rotates=None):
    """pypdf 后处理：设 /CropBox 与逐页 /Rotate（reportlab 无原生入口）。"""
    r = PdfReader(str(src))
    w = PdfWriter()
    w.append(r)
    for i, page in enumerate(w.pages):
        if crop is not None:
            page[NameObject("/CropBox")] = RectangleObject(crop)
        if rotates is not None:
            page[NameObject("/Rotate")] = NumberObject(rotates[i])
    with open(dst, "wb") as f:
        w.write(f)


# ---------------------------------------------------------------- manifest

def _round4(v):
    return round(float(v), 4)


def read_geometry(path: Path):
    """pypdf 独立几何（每页）——manifest 的跨引擎断言基准。"""
    r = PdfReader(str(path))
    pages = []
    for p in r.pages:
        media = [float(v) for v in p.mediabox]
        # pypdf 的 cropbox 属性对无 /CropBox 页回落 MediaBox 值——与
        # 引擎「crop ≡ media（容差 0.5pt）归 None」的干净语义对齐：
        # 只有真裁剪（与 MediaBox 差 > 0.5pt）才记录。
        crop = [float(v) for v in p.cropbox] if p.cropbox else None
        if crop is not None and all(
            abs(c - m) <= 0.5 for c, m in zip(crop, media)
        ):
            crop = None
        base = crop if crop else media
        w, h = base[2] - base[0], base[3] - base[1]
        rotate = int(p.get("/Rotate", 0) or 0)
        if rotate in (90, 270):
            w, h = h, w
        pages.append({
            "media_box": [_round4(v) for v in media],
            "crop_box": [_round4(v) for v in crop] if crop else None,
            "rotate": rotate,
            "display_size_pt": [_round4(w), _round4(h)],
        })
    return pages


BUILDERS = [
    ("chinese_embedded", build_chinese_embedded, {"ncc_min": 0.90, "ink_iou_min": 0.70},
     "嵌入 NotoSansSC 子集——两引擎字形一致（IoU 单列 0.70：文本页大量"
     "抗锯齿边缘格在软墨量两侧分布，实测基线 0.733）"),
    ("chinese_missing_font", build_chinese_missing_font, {"ncc_min": 0.80, "ink_iou_min": 0.60},
     "非嵌入 CID STSong-Light——两引擎字体回退形态漂移（低阈值单列，实测校准）"),
    ("scanned_gray", build_scanned_gray, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "整页灰度 JPEG 扫描件——零文本层，DCT 解码对照"),
    ("vector_graphics", build_vector_graphics, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "纯矢量线/矩形/圆/贝塞尔——抗锯齿形态对照"),
    ("transparency_blend", build_transparency_blend, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "ExtGState alpha 混合——半透明叠加形态对照"),
    ("cropped_mediabox", build_cropped_mediabox, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "MediaBox 842×596 + CropBox [101,51,741,545]——裁剪生效 + 显示域≠MediaBox"),
    ("rotation_matrix", build_rotation_matrix, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "/Rotate 90 与 180——方向特征像素断言（旋转正确显示）"),
    ("landscape_mixed", build_landscape_mixed, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "3 页真异 MediaBox（纵/横/方）——页尺寸混排"),
    ("multipage_120", build_multipage_120, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "120 页——多页确定性与跨文档面"),
    ("tiny_note", build_tiny_note, {"ncc_min": 0.90, "ink_iou_min": 0.72},
     "40×720 极窄页（aspect 18）——极端纵横比"),
]


def main():
    pdfmetrics.registerFont(TTFont("NotoSansSC", FONT_SC))
    fixtures = []
    for name, builder, parity, note in BUILDERS:
        path, meta = builder()
        geo = read_geometry(path)
        assert len(geo) == meta["pages"], f"{name}: 页数不符 {len(geo)} != {meta['pages']}"
        size = path.stat().st_size
        print(f"  {name}.pdf  {meta['pages']:>3}p  {size:>7}B  首页显示域 "
              f"{geo[0]['display_size_pt'][0]:.1f}×{geo[0]['display_size_pt'][1]:.1f}pt  "
              f"rotate={geo[0]['rotate']}")
        fixtures.append({
            "file": f"{name}.pdf",
            "notes": note,
            "probe_pages": meta["probe_pages"],
            "parity": parity,
            "pages": [{"page": i + 1, **g} for i, g in enumerate(geo)],
        })
    manifest = {
        "task": "#634 PDFium 正确显示专项——引擎层显示矩阵",
        "generator": "gen_display_matrix.py（确定性生成；reportlab 基础 + pypdf 后处理/独立几何）",
        "geometry_source": "pypdf 独立解析（跨引擎断言基准——不信任 pdfium 自报）",
        "poppler_reference": "pdftoppm -cropbox -gray -r <dpi>（显示域口径与 pdfium 对齐）",
        "fixtures": fixtures,
    }
    (DIR / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"manifest.json 写出（{len(fixtures)} 夹具）")


if __name__ == "__main__":
    main()
