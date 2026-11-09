#!/home/z/.venv/bin/python3
# -*- coding: utf-8 -*-
"""
build_corpus.py — #634 PDF 页面正确显示专项：显示链验证语料构建（单文件、可重复执行）

产出：
  1) 15 个真实 PDF 语料      → mdcore/mang-shell/tests/fixtures/display_corpus/*.pdf
  2) 参照渲染 PNG（证据，不入库） → /tmp/corpus_renders/
  3) reference_manifest.json   → 语料目录（pypdf 独立几何 + poppler 24×24 网格参照）
  4) calibration_report.json   → /tmp/corpus_renders/（pypdfium2 同 build 跨引擎校准与容差推荐）

关键约定：
  - 光栅宽度固定 640px（高度按比例），与 Rust 侧同宽渲染对齐；
  - render_w_pt/render_h_pt 由 pypdf 独立计算：取 CropBox → 应用 /Rotate 交换宽高（跨引擎几何断言基准）；
  - 网格 24×24：grid_mean=灰度均值（白=1.0），grid_ink=墨迹覆盖率（像素<200/255 判墨）；
    网格为行主序 [row][col]，row 0 = 渲染页顶部；
  - poppler 参照渲染命令在规格基础上追加两个必要对齐开关：
      * -cropbox                        —— poppler CLI 默认渲染 MediaBox，而 PDFium 默认 CropBox；
      * -scale-dimension-before-rotation —— 使 -scale-to-x 的 640px 作用在旋转后的可视宽度上，
        否则 /Rotate 90 页会得到 906×640（先按未旋转宽缩放再旋转）。
"""
import glob
import json
import math
import os
import random
import re
import subprocess
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

from reportlab.lib.pagesizes import A4 as A4_RL
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, NameObject, RectangleObject

import pypdfium2 as pdfium
import pypdfium2_raw.version as _pdfium_ver

# ---------------------------------------------------------------- 常量
REPO = "/home/z/my-project/mdcore"
CORPUS_DIR = os.path.join(REPO, "mang-shell/tests/fixtures/display_corpus")
RENDER_DIR = "/tmp/corpus_renders"
SCRIPT_TMP_PREFIX = "_tmp"          # 生成期中间文件统一前缀（放在 RENDER_DIR）

A4_W, A4_H = A4_RL                  # 595.2756 × 841.8898 pt
FONT_REG = "/usr/share/fonts/truetype/chinese/NotoSansSC-Regular.ttf"
ZH_FONT = "NotoSansSC"              # reportlab 注册名

RASTER_W = 640                      # 固定渲染宽（px）
GRID_N = 24                         # 网格边长
INK_T = 200                         # 墨迹阈值（<200/255 判墨）
GEOM_ANOMALY_PCT = 2.0              # 纵横比偏差 > 2% 记为几何异常
MAX_CORPUS_BYTES = 500 * 1024       # mixed/long_doc 体积上限

PDFIUM_BUILD = str(_pdfium_ver.PDFIUM_INFO.build)   # 期望 8149（chromium 159.0.8149）

PROBE_PAGES = {                     # 探针页（1 基）；坏文件不渲染
    "zh_embedded_font.pdf": [1, 2],
    "zh_missing_font.pdf": [1, 2],
    "scanned_page.pdf": [1, 2],
    "vector_graphics.pdf": [1],
    "transparency_clip.pdf": [1],
    "rotated_90.pdf": [1, 2],
    "rotated_180.pdf": [1],
    "landscape_mixed.pdf": [1, 2],
    "cropbox_offset.pdf": [1, 2],
    "cropbox_inset.pdf": [1, 2],
    "long_doc_120p.pdf": [1, 60, 119],
    "oversized_page.pdf": [1],
    "mixed_features.pdf": [1, 2, 3, 4],
}

# ---------------------------------------------------------------- 中文语料文本
ZH_PARA_1 = ("在多引擎渲染校准中，同一份文档会被 PDFium 与 Poppler 分别光栅化，"
             "再对 24×24 网格的灰度均值与墨迹覆盖率逐格比对，"
             "从而为跨引擎像素断言建立可复现的容差基准。本页字体以 TrueType 子集方式嵌入，"
             "任何缺字、乱码或查看器回退替换，都会在网格统计中表现为可定位的异常峰值。")
ZH_PARA_2 = ("本段混排中文标点（，。、；：？！“”‘’（）——《》）、阿拉伯数字（0123456789、3.14159、99.9%）、"
             "拉丁字母（Hello, World! ABCDEFG xyz）以及全角符号（％￥§№①②③），"
             "用于验证子集覆盖范围与基线一致性：标点占位、引号宽度、数字对齐，都是渲染回归的高发区域。")
ZH_PARA_3 = ("扫描页、矢量页、透明叠加页与旋转页各自独立成文件，又在综合样张中复合出现；"
             "一处几何回归——旋转交换宽高、裁剪框偏移、透明度合成顺序——应当能被网格参照数据"
             "定位到具体页面与区域，而不是淹没在整页均值里。")
ZH_PARA_4 = ("页面旋转（0°/90°/180°/270°）与裁剪框（CropBox）偏移会改变可视区域的宽高比："
             "渲染器必须先取裁剪框、再应用旋转、最后按目标宽度缩放，"
             "否则网格行列将整体错位。此段在两页中重复出现，用于跨页一致性交叉检查。")


# ---------------------------------------------------------------- 基础工具
def ensure_dirs():
    os.makedirs(CORPUS_DIR, exist_ok=True)
    os.makedirs(RENDER_DIR, exist_ok=True)


def clean_render_dir():
    """清空参照渲染目录里的旧产物（PNG/JPG/报告），保证可重复执行"""
    for pat in ("*.png", "*.jpg", "*.json"):
        for f in glob.glob(os.path.join(RENDER_DIR, pat)):
            os.remove(f)


def ensure_fonts():
    """注册中文字体（TTFont 子集嵌入）"""
    pdfmetrics.registerFont(TTFont(ZH_FONT, FONT_REG))


def chk(cond, msg):
    """验证断言：失败即抛错（生成脚本必须修到全过）"""
    if not cond:
        raise RuntimeError("[verify-fail] " + msg)


def approx(v, t, tol=0.06):
    return abs(v - t) <= tol


def box_eq(box, target, tol=0.06):
    return len(box) == len(target) and all(abs(a - b) <= tol for a, b in zip(box, target))


# ---------------------------------------------------------------- 绘制助手（reportlab 场景）
def draw_zh_paragraph(c, text, x, y, max_w, font=ZH_FONT, size=11, leading=18.5):
    """中英混排逐字贪心换行；返回下一段可用 y 坐标"""
    c.setFont(font, size)
    line = ""
    for ch in text:
        trial = line + ch
        if line and pdfmetrics.stringWidth(trial, font, size) > max_w:
            c.drawString(x, y, line)
            y -= leading
            line = ch
        else:
            line = trial
    if line:
        c.drawString(x, y, line)
        y -= leading
    return y


def draw_zh_page(c, title, paras, footer):
    """中文正文页：标题 + 段落 + 标点/数字混排行 + 页脚"""
    c.setFillColorRGB(0.09, 0.09, 0.11)
    c.setFont(ZH_FONT, 19)
    c.drawString(72, A4_H - 78, title)
    c.setStrokeColorRGB(0.25, 0.25, 0.30)
    c.setLineWidth(1.2)
    c.line(72, A4_H - 92, A4_W - 72, A4_H - 92)
    y = A4_H - 130
    for para in paras:
        y = draw_zh_paragraph(c, para, 72, y, A4_W - 144, size=11, leading=18.5) - 10
    c.setFillColorRGB(0.1, 0.1, 0.1)
    c.setFont(ZH_FONT, 12)
    c.drawString(72, 92, "0123456789 ①②③ 3.14159 99.9% ￥1,234.56 “引号”《书名》——（括号）")
    c.setFont(ZH_FONT, 9)
    c.setFillColorRGB(0.35, 0.35, 0.40)
    c.drawString(72, 56, footer)
    c.drawRightString(A4_W - 72, 56, "mdcore 显示语料")


def draw_vector_scene(c):
    """矢量图形场景：贝塞尔填充/描边、星形多边形、线宽渐变、虚线圆、同心圆组"""
    # 1) 贝塞尔闭合路径：填充 + 描边（有机 blob）
    c.setFillColorRGB(0.25, 0.45, 0.78)
    c.setStrokeColorRGB(0.08, 0.12, 0.35)
    c.setLineWidth(3)
    p = c.beginPath()
    p.moveTo(70, 700)
    p.curveTo(170, 810, 430, 790, 490, 700)
    p.curveTo(550, 610, 430, 545, 300, 585)
    p.curveTo(175, 620, 40, 620, 70, 700)
    p.close()
    c.drawPath(p, stroke=1, fill=1)
    # 2) 开放贝塞尔粗线（圆头端点）
    c.setStrokeColorRGB(0.82, 0.18, 0.18)
    c.setLineWidth(13)
    c.setLineCap(1)
    p = c.beginPath()
    p.moveTo(64, 520)
    p.curveTo(210, 430, 330, 620, 530, 490)
    c.drawPath(p, stroke=1, fill=0)
    # 3) 十角星多边形（凹多边形填充）
    cx, cy, R = 478, 560, 70
    p = c.beginPath()
    for i in range(20):
        ang = -math.pi / 2 + i * math.pi / 10
        rad = R if i % 2 == 0 else R * 0.45
        x, y = cx + rad * math.cos(ang), cy + rad * math.sin(ang)
        (p.moveTo if i == 0 else p.lineTo)(x, y)
    p.close()
    c.setFillColorRGB(0.98, 0.80, 0.10)
    c.setStrokeColorRGB(0.50, 0.35, 0.0)
    c.setLineWidth(2)
    c.drawPath(p, stroke=1, fill=1)
    # 4) 线宽渐变组：0.4 → 14 pt
    c.setStrokeColorRGB(0.12, 0.12, 0.15)
    y_base = 430
    for i, lw in enumerate((0.4, 0.8, 1.5, 3, 6, 14)):
        c.setLineWidth(lw)
        c.line(70, y_base - i * 24, 470 + i * 18, y_base - i * 24)
    # 5) 虚线大圆
    c.setDash(7, 4)
    c.setLineWidth(2.4)
    c.setStrokeColorRGB(0.10, 0.50, 0.55)
    c.circle(470, 170, 90, stroke=1, fill=0)
    c.setDash()
    # 6) 同心圆组：填充/描边交替
    for i in range(6):
        r = 150 - i * 23
        if i % 2 == 0:
            c.setFillColorRGB(0.93, 0.93, 0.96)
            c.setStrokeColorRGB(0.15, 0.45, 0.20)
            c.setLineWidth(1.8)
            c.circle(150, 250, r, stroke=1, fill=1)
        else:
            c.setStrokeColorRGB(0.55, 0.10, 0.55)
            c.setLineWidth(4)
            c.circle(150, 250, r, stroke=1, fill=0)
    # 7) 三角形 + 六边形描边
    c.setStrokeColorRGB(0.20, 0.20, 0.60)
    c.setLineWidth(5)
    c.line(330, 60, 430, 240)
    c.line(430, 240, 230, 240)
    c.line(230, 240, 330, 60)
    p = c.beginPath()
    for i in range(6):
        ang = math.pi / 6 + i * math.pi / 3
        x, y = 470 + 60 * math.cos(ang), 60 + 60 * math.sin(ang)
        (p.moveTo if i == 0 else p.lineTo)(x, y)
    p.close()
    c.drawPath(p, stroke=1, fill=0)


def draw_transparency_scene(c):
    """透明 + 裁剪场景：/ca /CA 半透明叠加、clip path 内绘制、图元跨界"""
    # 1) 三个半透明色块互相叠加（填充 alpha /ca）
    c.saveState()
    c.setFillAlpha(0.35)
    c.setFillColorRGB(0.90, 0.10, 0.10)
    c.rect(70, 500, 240, 240, stroke=0, fill=1)
    c.setFillAlpha(0.45)
    c.setFillColorRGB(0.10, 0.60, 0.10)
    c.rect(210, 500, 240, 240, stroke=0, fill=1)
    c.setFillAlpha(0.55)
    c.setFillColorRGB(0.10, 0.20, 0.90)
    c.rect(150, 600, 240, 240, stroke=0, fill=1)
    c.restoreState()
    # 2) 圆形裁剪：跨界矩形 + 粗斜线 + 半透明圆，只应在圆内可见
    c.saveState()
    cp = c.beginPath()
    cp.circle(430, 300, 130)
    c.clipPath(cp, stroke=0, fill=0)
    c.setFillColorRGB(0.95, 0.75, 0.10)
    c.rect(430 - 170, 300 - 170, 340, 340, stroke=0, fill=1)   # 跨裁剪边界的实心矩形
    c.setStrokeColorRGB(0.20, 0.20, 0.20)
    c.setLineWidth(10)
    c.line(430 - 180, 300 - 150, 430 + 180, 300 + 150)
    c.line(430 + 180, 300 - 150, 430 - 180, 300 + 150)
    c.setFillAlpha(0.50)
    c.setFillColorRGB(0.60, 0.10, 0.60)
    c.circle(430, 300, 90, stroke=0, fill=1)
    c.setFillAlpha(1)
    c.restoreState()
    # 3) 三角形裁剪 + 跨界网格线
    c.saveState()
    tp = c.beginPath()
    tp.moveTo(110, 70)
    tp.lineTo(360, 70)
    tp.lineTo(235, 300)
    tp.close()
    c.clipPath(tp, stroke=0, fill=0)
    c.setLineWidth(1.2)
    c.setStrokeColorRGB(0.30, 0.30, 0.50)
    for i in range(31):
        c.line(100 + i * 10, 50, 100 + i * 10, 320)
    for j in range(31):
        c.line(90, 60 + j * 9, 400, 60 + j * 9)
    c.restoreState()
    # 4) 半透明色带横贯 + 半透明粗描边线（/CA）
    c.saveState()
    c.setFillAlpha(0.25)
    c.setFillColorRGB(0.20, 0.80, 0.30)
    c.rect(40, 425, A4_W - 80, 60, stroke=0, fill=1)
    c.setStrokeAlpha(0.65)
    c.setStrokeColorRGB(0.85, 0.40, 0.00)
    c.setLineWidth(26)
    c.line(60, 455, A4_W - 60, 455)
    c.restoreState()


def draw_orientation_page(c, num, note):
    """方向可辨的大号页码页（旋转语料用）：巨大数字 + 顶部箭头 + TOP/BOTTOM 标记"""
    W, H = c._pagesize
    c.setFillColorRGB(0.08, 0.08, 0.08)
    c.setFont("Helvetica-Bold", 300)
    c.drawCentredString(W / 2, H / 2 - 105, str(num))
    c.setFont("Helvetica-Bold", 34)
    c.drawCentredString(W / 2, H - 110, "TOP")
    c.drawCentredString(W / 2, 66, "BOTTOM")
    # 顶部红色上箭头（旋转后指向可辨）
    p = c.beginPath()
    p.moveTo(W / 2, H - 150)
    p.lineTo(W / 2 - 42, H - 225)
    p.lineTo(W / 2 + 42, H - 225)
    p.close()
    c.setFillColorRGB(0.85, 0.20, 0.20)
    c.drawPath(p, stroke=0, fill=1)
    # 边框 + 说明
    c.setLineWidth(6)
    c.setStrokeColorRGB(0.10, 0.10, 0.40)
    c.rect(30, 30, W - 60, H - 60, stroke=1, fill=0)
    c.setFont("Helvetica", 16)
    c.setFillColorRGB(0.1, 0.1, 0.1)
    c.drawString(48, 52, note)
    c.drawRightString(W - 48, H - 88, "ORIENTATION MARKER")


def draw_fullbleed_page(c, num, center_label):
    """CropBox 语料页：内容铺满整个 MediaBox（网格/边框/四角字大部分落在裁剪区外）"""
    W, H = A4_W, A4_H
    c.setFillColorRGB(0.965, 0.965, 0.97)
    c.rect(0, 0, W, H, stroke=0, fill=1)
    c.setStrokeColorRGB(0.72, 0.72, 0.78)
    c.setLineWidth(0.7)
    step = 48
    x = 0
    while x <= W:
        c.line(x, 0, x, H)
        x += step
    y = 0
    while y <= H:
        c.line(0, y, W, y)
        y += step
    # 页面最外沿粗框：正常渲染（取 CropBox）时不可见
    c.setStrokeColorRGB(0.10, 0.10, 0.50)
    c.setLineWidth(14)
    c.rect(16, 16, W - 32, H - 32, stroke=1, fill=0)
    # 四角大字：全部落在裁剪区之外
    c.setFillColorRGB(0.85, 0.10, 0.10)
    c.setFont("Helvetica-Bold", 30)
    c.drawString(28, 24, "CORNER-LL")
    c.drawString(28, H - 48, "CORNER-UL")
    c.drawRightString(W - 28, 24, "CORNER-LR")
    c.drawRightString(W - 28, H - 48, "CORNER-UR")
    # 中央大页码 + 说明（裁剪区内，必须可见）
    c.setFillColorRGB(0.05, 0.05, 0.20)
    c.setFont("Helvetica-Bold", 150)
    c.drawCentredString(W / 2, H / 2 - 55, str(num))
    c.setFont("Helvetica", 22)
    c.drawCentredString(W / 2, H / 2 - 110, center_label)


def _post_process(tmp_path, out_path, rot_by_page=None, cropbox=None):
    """pypdf 后处理：按页设置 /Rotate 与 /CropBox 后另存"""
    writer = PdfWriter(clone_from=tmp_path)
    for i, pg in enumerate(writer.pages):
        if rot_by_page and i in rot_by_page:
            pg.rotation = rot_by_page[i]
        if cropbox is not None:
            pg[NameObject("/CropBox")] = RectangleObject([float(v) for v in cropbox])
    writer.write(out_path)


# ---------------------------------------------------------------- 15 个语料生成函数
def _gen_zh_content(path):
    """中文嵌入正文内容（zh_embedded_font 与 zh_missing_font 共用同一版式）"""
    c = canvas.Canvas(path, pagesize=A4_RL)
    c.setTitle("中文嵌入字体样张")
    draw_zh_page(c, "中文嵌入字体测试 · 第 1 页", [ZH_PARA_1, ZH_PARA_2, ZH_PARA_3],
                 "zh_embedded_font.pdf · page 1/2 · TrueType 子集嵌入")
    c.showPage()
    draw_zh_page(c, "中文嵌入字体测试 · 第 2 页", [ZH_PARA_4, ZH_PARA_2, ZH_PARA_1, ZH_PARA_3],
                 "zh_embedded_font.pdf · page 2/2 · TrueType 子集嵌入")
    c.showPage()
    c.save()


def gen_zh_embedded_font(path):
    _gen_zh_content(path)


def _dfs_page_objects(root, max_depth=40):
    """对象图 DFS（带环保护），产出遍历到的所有 DictionaryObject"""
    seen = set()
    stack = [(root, 0)]
    while stack:
        obj, d = stack.pop()
        if d > max_depth:
            continue
        if isinstance(obj, IndirectObject):
            try:
                obj = obj.get_object()
            except Exception:
                continue
        oid = id(obj)
        if oid in seen:
            continue
        seen.add(oid)
        if isinstance(obj, DictionaryObject):
            yield obj
            for v in list(obj.values()):
                stack.append((v, d + 1))
        elif isinstance(obj, (ArrayObject, list)):
            for v in list(obj):
                stack.append((v, d + 1))


def _dfs_strip_fontfiles(root):
    """删除对象图中所有 /FontFile* 键（嵌入字体程序）"""
    seen = set()
    removed = []
    stack = [root]
    while stack:
        obj = stack.pop()
        if isinstance(obj, IndirectObject):
            try:
                obj = obj.get_object()
            except Exception:
                continue
        oid = id(obj)
        if oid in seen:
            continue
        seen.add(oid)
        if isinstance(obj, DictionaryObject):
            for k in [k for k in obj.keys() if str(k).startswith("/FontFile")]:
                del obj[k]
                removed.append(str(k))
            for v in list(obj.values()):
                stack.append(v)
        elif isinstance(obj, (ArrayObject, list)):
            for v in list(obj):
                stack.append(v)
    return removed


def gen_zh_missing_font(path):
    """缺失字体语料：同版式生成 → pypdf 剥掉 FontFile2，文本与 ToUnicode 保留"""
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_zh_src.pdf")
    _gen_zh_content(tmp)
    # 方案一：clone 后删除引用（不可达对象不应再被写出）
    writer = PdfWriter(clone_from=tmp)
    removed = _dfs_strip_fontfiles(writer.root_object)  # 根目录对象图 DFS，删所有 /FontFile*
    writer.write(path)
    raw = open(path, "rb").read()
    if b"FontFile" in raw:
        # 方案二（兜底）：全新 writer 逐页 add_page，仅复制可达对象后再删一次
        reader = PdfReader(tmp)
        writer2 = PdfWriter()
        for pg in reader.pages:
            writer2.add_page(pg)
        _dfs_strip_fontfiles(writer2.root_object)
        writer2.write(path)
        raw = open(path, "rb").read()
    chk(b"FontFile" not in raw, f"zh_missing_font 仍残留字体程序字节（removed={removed}）")
    os.remove(tmp)


def _scan_image(page_no, seed):
    """PIL 绘制“扫描件”灰度位图：中文标题/表格/印章圆圈/条码区 + 噪点 + 轻微歪斜"""
    random.seed(seed)
    W, H = 1240, 1754
    img = Image.new("L", (W, H), 252)
    d = ImageDraw.Draw(img)
    ink = 35
    f_title = ImageFont.truetype(FONT_REG, 56)
    f_body = ImageFont.truetype(FONT_REG, 34)
    f_small = ImageFont.truetype(FONT_REG, 26)
    if page_no == 1:
        title = "机密文件·扫描件（第一页）"
        meta = ["收文编号：A-2024-001", "密级：机密★保存一年", "扫描日期：2024-06-30", "页码：1 / 2"]
        rows = [("文件名称", "渲染管线显示验证语料"), ("份数", "2 份"), ("经办", "张 三"),
                ("审核", "李 四"), ("备注", "原件与扫描件一并归档")]
    else:
        title = "机密文件·扫描件（第二页）"
        meta = ["收文编号：A-2024-002", "密级：内部▲限阅", "扫描日期：2024-06-30", "页码：2 / 2"]
        rows = [("对应单号", "A-2024-001 附件"), ("页数", "共 2 页"), ("经办", "王 五"),
                ("审核", "赵 六"), ("备注", "本页为扫描影像，无文本层")]
    # 标题 + 分隔线
    d.text(((W - d.textlength(title, font=f_title)) / 2, 84), title, font=f_title, fill=ink)
    d.line((120, 168, W - 120, 168), fill=90, width=3)
    # 元信息行
    y = 204
    for line in meta:
        d.text((120, y), line, font=f_body, fill=ink)
        y += 52
    # 表格（表头 + 数据行）
    tx0, tx1, mid = 110, 1130, 560
    row_h = 78
    ty0 = y + 26
    d.rectangle([tx0, ty0, tx1, ty0 + row_h * (len(rows) + 1)], outline=ink, width=3)
    d.line((mid, ty0, mid, ty0 + row_h * (len(rows) + 1)), fill=ink, width=3)
    yy = ty0
    for a, b in [("字段", "内容")] + rows:
        d.text((tx0 + 18, yy + 22), a, font=f_body, fill=ink)
        d.text((mid + 18, yy + 22), b, font=f_body, fill=ink)
        yy += row_h
        if yy < ty0 + row_h * (len(rows) + 1):
            d.line((tx0, yy, tx1, yy), fill=ink, width=2)
    # 印章：双圈 + 五角星
    cx, cy, r = (980, 1350, 110) if page_no == 1 else (300, 1350, 110)
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=ink, width=7)
    r2 = r - 16
    d.ellipse([cx - r2, cy - r2, cx + r2, cy + r2], outline=ink, width=3)
    star = []
    for i in range(10):
        ang = -math.pi / 2 + i * math.pi / 5
        rad = (r2 - 14) if i % 2 == 0 else (r2 - 14) * 0.45
        star.append((cx + rad * math.cos(ang), cy + rad * math.sin(ang)))
    d.polygon(star, fill=ink)
    tag = "专用章" if page_no == 1 else "附件章"
    d.text((cx - d.textlength(tag, font=f_small) / 2, cy + r + 8), tag, font=f_small, fill=ink)
    # 条码区
    d.text((120, 1560), "条码区 SN-2024-%03d" % page_no, font=f_small, fill=ink)
    rng = random.Random(5000 + page_no)
    x = 120
    while x < 760:
        bw = rng.choice((2, 3, 5, 8))
        if rng.random() < 0.6:
            d.rectangle([x, 1606, x + bw, 1668], fill=ink)
        x += bw + rng.choice((2, 3, 4))
    # 签名区
    d.text((820, 1620), "签名：", font=f_body, fill=ink)
    d.line((910, 1626, 1110, 1626), fill=ink, width=3)
    # 噪点 + 轻微模糊 + 0.35° 歪斜（模拟扫描）
    noise = Image.effect_noise((W, H), 26)
    img = Image.blend(img, noise, 0.05)
    img = img.filter(ImageFilter.GaussianBlur(0.4))
    img = img.rotate(0.35, resample=Image.BICUBIC, fillcolor=249, expand=True)
    x0 = (img.width - W) // 2
    y0 = (img.height - H) // 2
    img = img.crop((x0, y0, x0 + W, y0 + H))
    # 少量扫描污点
    d2 = ImageDraw.Draw(img)
    for _ in range(40):
        px, py = random.randint(0, W - 4), random.randint(0, H - 4)
        rr = random.randint(1, 3)
        d2.ellipse([px, py, px + rr, py + rr], fill=random.randint(80, 140))
    return img


def gen_scanned_page(path):
    c = canvas.Canvas(path, pagesize=A4_RL)
    c.setTitle("扫描件样张（无文本层）")
    for pno in (1, 2):
        img = _scan_image(pno, seed=777 + pno)
        jpg = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + f"_scan_p{pno}.jpg")
        img.save(jpg, "JPEG", quality=58, optimize=True)
        c.drawImage(jpg, 0, 0, A4_W, A4_H)   # 整页铺满（DCT 直通嵌入）
        c.showPage()
    c.save()
    for pno in (1, 2):
        os.remove(os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + f"_scan_p{pno}.jpg"))


def gen_vector_graphics(path):
    c = canvas.Canvas(path, pagesize=A4_RL)
    draw_vector_scene(c)
    c.showPage()
    c.save()


def gen_transparency_clip(path):
    c = canvas.Canvas(path, pagesize=A4_RL)
    draw_transparency_scene(c)
    c.showPage()
    c.save()


def gen_rotated_90(path):
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_rot90.pdf")
    c = canvas.Canvas(tmp, pagesize=A4_RL)
    draw_orientation_page(c, 1, "PAGE 1 - /Rotate 90 expected")
    c.showPage()
    draw_orientation_page(c, 2, "PAGE 2 - /Rotate 270 expected")
    c.showPage()
    c.save()
    _post_process(tmp, path, rot_by_page={0: 90, 1: 270})
    os.remove(tmp)


def gen_rotated_180(path):
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_rot180.pdf")
    c = canvas.Canvas(tmp, pagesize=A4_RL)
    draw_orientation_page(c, 1, "PAGE 1 - /Rotate 180 expected")
    c.showPage()
    c.save()
    _post_process(tmp, path, rot_by_page={0: 180})
    os.remove(tmp)


def gen_landscape_mixed(path):
    """3 页：竖 A4 / 横 MediaBox(842×595)+/Rotate 90 / 竖 A4"""
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_landscape.pdf")
    c = canvas.Canvas(tmp, pagesize=A4_RL)
    draw_orientation_page(c, 1, "PAGE 1 - portrait MediaBox 595x842 - Rotate 0")
    c.showPage()
    c.setPageSize((A4_H, A4_W))       # 第 2 页：横版 MediaBox
    draw_orientation_page(c, 2, "PAGE 2 - landscape MediaBox 842x595 - /Rotate 90")
    c.showPage()
    c.setPageSize(A4_RL)
    draw_orientation_page(c, 3, "PAGE 3 - portrait MediaBox 595x842 - Rotate 0")
    c.showPage()
    c.save()
    _post_process(tmp, path, rot_by_page={1: 90})
    os.remove(tmp)


def gen_cropbox_offset(path):
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_cropo.pdf")
    c = canvas.Canvas(tmp, pagesize=A4_RL)
    draw_fullbleed_page(c, 1, "CROPBOX OFFSET [72,72,523,770]")
    c.showPage()
    draw_fullbleed_page(c, 2, "CROPBOX OFFSET [72,72,523,770]")
    c.showPage()
    c.save()
    _post_process(tmp, path, cropbox=(72, 72, 523, 770))
    os.remove(tmp)


def gen_cropbox_inset(path):
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_cropi.pdf")
    c = canvas.Canvas(tmp, pagesize=A4_RL)
    draw_fullbleed_page(c, 1, "CROPBOX INSET [108,54,487,738]")
    c.showPage()
    draw_fullbleed_page(c, 2, "CROPBOX INSET [108,54,487,738]")
    c.showPage()
    c.save()
    _post_process(tmp, path, cropbox=(108, 54, 487, 738))
    os.remove(tmp)


def long_doc_paragraphs(n):
    """长文档每页段落：页码/流水号/日期随页变化，保证每页唯一可辨"""
    rng = random.Random(90000 + n)
    qty = rng.randint(3, 9)
    return [
        f"第 {n} 页段落一：本页用于多页文档翻页与滚动性能验证，页码在任意缩放级别下必须清晰可辨；"
        f"本组样本数量为 {qty}，流水号 SN-{n:04d}-{rng.randint(100, 999)}。",
        f"第 {n} 页段落二：The quick brown fox jumps over the lazy dog. 0123456789 12.5% ±0.5℃，"
        f"金额 {rng.randint(1000, 9999):,}.00 元，日期 2024-06-{(n % 28) + 1:02d}。",
        "段落三（每页相同）：裁剪、旋转、透明叠加与扫描样张见综合文档；长文档场景下内存占用应保持平稳，不应随页数线性恶化。",
    ]


def gen_long_doc_120p(path):
    c = canvas.Canvas(path, pagesize=A4_RL)
    c.setTitle("120 页长文档")
    for n in range(1, 121):
        c.setFont(ZH_FONT, 10)
        c.setFillColorRGB(0.25, 0.25, 0.25)
        c.drawString(72, 800, "长文档渲染测试 · 每页页码唯一可辨")
        c.drawRightString(A4_W - 72, 800, f"{n} / 120")
        c.setLineWidth(0.8)
        c.setStrokeColorRGB(0.6, 0.6, 0.6)
        c.line(72, 793, A4_W - 72, 793)
        c.setFillColorRGB(0.08, 0.08, 0.08)
        c.setFont("Helvetica-Bold", 96)
        c.drawCentredString(A4_W / 2, 655, str(n))
        c.setFillColorRGB(0.1, 0.1, 0.1)
        y = 596
        for para in long_doc_paragraphs(n):
            y = draw_zh_paragraph(c, para, 72, y, A4_W - 144, size=11, leading=17) - 6
        c.setFont(ZH_FONT, 9)
        c.setFillColorRGB(0.4, 0.4, 0.4)
        c.drawString(72, 50, "mdcore 显示链语料 · build_corpus.py 生成")
        c.drawRightString(A4_W - 72, 50, f"第 {n} 页")
        c.showPage()
    c.save()


def gen_oversized_page(path):
    W, H = 2000, 2800
    c = canvas.Canvas(path, pagesize=(W, H))
    c.setTitle("超大页面样张")
    c.setFillColorRGB(0.98, 0.98, 1.0)
    c.rect(0, 0, W, H, stroke=0, fill=1)
    c.setStrokeColorRGB(0.10, 0.10, 0.40)
    c.setLineWidth(18)
    c.rect(60, 60, W - 120, H - 120, stroke=1, fill=0)
    for i in range(8):
        c.setLineWidth(6 + i * 3)
        c.setStrokeColorRGB(min(0.1 + i * 0.06, 1.0), 0.2, 0.5)
        c.circle(W / 2, H * 0.62, 220 + i * 90, stroke=1, fill=0)
    c.setStrokeColorRGB(0.80, 0.20, 0.20)
    c.setLineWidth(10)
    c.line(80, 80, W - 80, H - 80)
    c.line(W - 80, 80, 80, H - 80)
    c.setFillColorRGB(0.05, 0.05, 0.25)
    c.setFont("Helvetica-Bold", 170)
    c.drawCentredString(W / 2, 500, "OVERSIZE 2000 x 2800 pt")
    c.setFont(ZH_FONT, 120)
    c.drawCentredString(W / 2, 300, "超大页面测试")
    c.showPage()
    c.save()


def gen_damaged_truncated(path, src):
    """截断尾部 30% 字节的坏文件（源：zh_embedded_font.pdf）"""
    data = open(src, "rb").read()
    keep = int(len(data) * 0.7)
    with open(path, "wb") as f:
        f.write(data[:keep])


def gen_damaged_garbage(path):
    """%PDF 头部 + 全随机字节（512KB），可复现（固定种子）"""
    rng = random.Random(0xC0FFEE)
    head = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"
    body = rng.randbytes(512 * 1024 - len(head))
    with open(path, "wb") as f:
        f.write(head + body)


def gen_mixed_features(path):
    """综合合体：4 页 = 中文文字 / 矢量 / 透明 / 旋转 90，页尺寸统一 A4"""
    tmp = os.path.join(RENDER_DIR, SCRIPT_TMP_PREFIX + "_mixed.pdf")
    c = canvas.Canvas(tmp, pagesize=A4_RL)
    c.setTitle("综合特征样张")
    # 页 1：中文嵌入文字
    draw_zh_page(c, "综合样张 · 页 1 · 中文嵌入文字", [ZH_PARA_1, ZH_PARA_3],
                 "mixed_features.pdf · page 1/4 · 中文嵌入")
    c.showPage()
    # 页 2：矢量图形
    draw_vector_scene(c)
    c.showPage()
    # 页 3：透明叠加 + 裁剪
    draw_transparency_scene(c)
    c.showPage()
    # 页 4：大号页码（后处理 /Rotate 90）
    draw_orientation_page(c, 4, "PAGE 4 - /Rotate 90 expected")
    c.showPage()
    c.save()
    _post_process(tmp, path, rot_by_page={3: 90})
    os.remove(tmp)


# ---------------------------------------------------------------- pypdf 独立验证
def page_verify_info(pg, deep_text=True):
    """单页特性探测：几何 / 旋转 / 文本 / 图片 / 嵌入字体 / 透明 ExtGState"""
    info = {}
    mb = pg.mediabox
    info["mediabox"] = [round(float(v), 2) for v in mb]
    try:
        cb = pg.cropbox
        info["cropbox"] = [round(float(v), 2) for v in cb]
    except Exception:
        info["cropbox"] = info["mediabox"]
    info["rotation"] = int(pg.rotation or 0)
    txt = ""
    if deep_text:
        try:
            txt = pg.extract_text() or ""
        except Exception:
            txt = ""
    info["has_text"] = bool(txt.strip())
    info["has_cjk_text"] = any("\u4e00" <= ch <= "\u9fff" for ch in txt)
    info["text_len"] = len(txt.strip())
    # 图片：资源 XObject 中是否有 /Subtype /Image
    has_img = False
    res = pg.get("/Resources")
    if res is not None:
        res = res.get_object()
        xo = res.get("/XObject")
        if xo is not None:
            xo = xo.get_object()
            for _, obj in xo.items():
                o = obj.get_object()
                if isinstance(o, DictionaryObject) and o.get("/Subtype") == "/Image":
                    has_img = True
                    break
    info["has_images"] = has_img
    # 嵌入字体程序 / 透明 ExtGState：对象图 DFS
    emb = False
    alpha = False
    for d in _dfs_page_objects(pg.get_object()):
        for k in d.keys():
            if str(k).startswith("/FontFile"):
                emb = True
        for key in ("/ca", "/CA"):
            if key in d:
                try:
                    if float(d[key]) < 1.0:
                        alpha = True
                except Exception:
                    pass
    info["embedded_fonts"] = emb
    info["alpha_extgstate"] = alpha
    return info


def render_geometry_pt(pg):
    """跨引擎几何断言基准：CropBox 尺寸 → 应用 /Rotate（90/270 交换宽高）"""
    cb = pg.cropbox
    w, h = float(cb.width), float(cb.height)
    rot = int(pg.rotation or 0) % 360
    if rot in (90, 270):
        w, h = h, w
    return w, h, rot


def verify_corpus():
    """逐文件 pypdf 独立验证（验证失败即抛错，修到全过）"""
    A4_BOX = [0.0, 0.0, round(A4_W, 2), round(A4_H, 2)]
    results = {}

    def V(name):
        path = os.path.join(CORPUS_DIR, name)
        return path, PdfReader(path)

    # 1) zh_embedded_font：2 页全嵌入 + 中文文本 + 无图片
    path, r = V("zh_embedded_font.pdf")
    chk(len(r.pages) == 2, "zh_embedded_font 页数 != 2")
    for i, pg in enumerate(r.pages):
        inf = page_verify_info(pg)
        chk(inf["embedded_fonts"], f"zh_embedded_font p{i} 未检出嵌入字体")
        chk(inf["has_text"] and inf["has_cjk_text"], f"zh_embedded_font p{i} 中文文本缺失")
        chk(not inf["has_images"], f"zh_embedded_font p{i} 不应有图片")
        chk(inf["rotation"] == 0 and box_eq(inf["mediabox"], A4_BOX), f"zh_embedded_font p{i} 几何异常")
    results["zh_embedded_font.pdf"] = "2 页，TrueType 子集嵌入+中文可提取，无图片，A4"

    # 2) zh_missing_font：2 页无嵌入字体程序 + 文本仍在
    path, r = V("zh_missing_font.pdf")
    chk(len(r.pages) == 2, "zh_missing_font 页数 != 2")
    for i, pg in enumerate(r.pages):
        inf = page_verify_info(pg)
        chk(not inf["embedded_fonts"], f"zh_missing_font p{i} 仍存在嵌入字体程序")
        chk(inf["has_text"] and inf["has_cjk_text"], f"zh_missing_font p{i} 文本内容丢失")
        chk(inf["text_len"] > 100, f"zh_missing_font p{i} 文本过短")
    results["zh_missing_font.pdf"] = "2 页，无 FontFile* 但中文文本可提取"

    # 3) scanned_page：2 页整页图片 + 无文本层
    path, r = V("scanned_page.pdf")
    chk(len(r.pages) == 2, "scanned_page 页数 != 2")
    for i, pg in enumerate(r.pages):
        inf = page_verify_info(pg)
        chk(inf["has_images"], f"scanned_page p{i} 未检出图片")
        chk(not inf["has_text"], f"scanned_page p{i} 不应有文本层")
        chk(inf["rotation"] == 0 and box_eq(inf["mediabox"], A4_BOX), f"scanned_page p{i} 几何异常")
    results["scanned_page.pdf"] = "2 页，每页含 1 张整页图片（DCTDecode），无文本"

    # 4) vector_graphics：1 页纯矢量
    path, r = V("vector_graphics.pdf")
    inf = page_verify_info(r.pages[0])
    chk(len(r.pages) == 1, "vector_graphics 页数 != 1")
    chk(not inf["has_text"] and not inf["has_images"], "vector_graphics 应为纯矢量页")
    chk(box_eq(inf["mediabox"], A4_BOX), "vector_graphics 应为 A4")
    results["vector_graphics.pdf"] = "1 页，贝塞尔/多边形/线宽/虚线纯矢量，无文本无图片"

    # 5) transparency_clip：1 页 ExtGState alpha
    path, r = V("transparency_clip.pdf")
    inf = page_verify_info(r.pages[0])
    chk(len(r.pages) == 1, "transparency_clip 页数 != 1")
    chk(inf["alpha_extgstate"], "transparency_clip 未检出 /ca 或 /CA < 1")
    chk(not inf["has_text"] and not inf["has_images"], "transparency_clip 应无文本无图片")
    results["transparency_clip.pdf"] = "1 页，/ca /CA 半透明 + clip path，A4"

    # 6) rotated_90：/Rotate [90, 270]
    path, r = V("rotated_90.pdf")
    chk(len(r.pages) == 2, "rotated_90 页数 != 2")
    chk(page_verify_info(r.pages[0])["rotation"] == 90, "rotated_90 p1 /Rotate != 90")
    chk(page_verify_info(r.pages[1])["rotation"] == 270, "rotated_90 p2 /Rotate != 270")
    results["rotated_90.pdf"] = "2 页 A4，/Rotate 90 / 270，大号页码方向可辨"

    # 7) rotated_180
    path, r = V("rotated_180.pdf")
    chk(len(r.pages) == 1, "rotated_180 页数 != 1")
    chk(page_verify_info(r.pages[0])["rotation"] == 180, "rotated_180 /Rotate != 180")
    results["rotated_180.pdf"] = "1 页 A4，/Rotate 180"

    # 8) landscape_mixed：页 2 横版 MediaBox + /Rotate 90
    path, r = V("landscape_mixed.pdf")
    chk(len(r.pages) == 3, "landscape_mixed 页数 != 3")
    infos = [page_verify_info(pg) for pg in r.pages]
    chk(box_eq(infos[0]["mediabox"], A4_BOX) and infos[0]["rotation"] == 0, "landscape_mixed p1 应为竖版 A4 无旋转")
    chk(box_eq(infos[1]["mediabox"], [0.0, 0.0, round(A4_H, 2), round(A4_W, 2)]), "landscape_mixed p2 MediaBox 应为 842×595")
    chk(infos[1]["rotation"] == 90, "landscape_mixed p2 /Rotate != 90")
    chk(box_eq(infos[2]["mediabox"], A4_BOX) and infos[2]["rotation"] == 0, "landscape_mixed p3 应为竖版 A4 无旋转")
    results["landscape_mixed.pdf"] = "3 页竖/横/竖，页 2 MediaBox 842×595 + /Rotate 90"

    # 9) cropbox_offset
    path, r = V("cropbox_offset.pdf")
    chk(len(r.pages) == 2, "cropbox_offset 页数 != 2")
    for i, pg in enumerate(r.pages):
        inf = page_verify_info(pg)
        chk(box_eq(inf["mediabox"], A4_BOX), f"cropbox_offset p{i} MediaBox 应为 A4")
        chk(box_eq(inf["cropbox"], [72, 72, 523, 770]), f"cropbox_offset p{i} CropBox != [72,72,523,770]")
    results["cropbox_offset.pdf"] = "2 页，MediaBox A4，CropBox [72,72,523,770] 偏移"

    # 10) cropbox_inset
    path, r = V("cropbox_inset.pdf")
    chk(len(r.pages) == 2, "cropbox_inset 页数 != 2")
    for i, pg in enumerate(r.pages):
        inf = page_verify_info(pg)
        chk(box_eq(inf["cropbox"], [108, 54, 487, 738]), f"cropbox_inset p{i} CropBox != [108,54,487,738]")
    results["cropbox_inset.pdf"] = "2 页，CropBox [108,54,487,738] 四周不对称内缩"

    # 11) long_doc_120p：120 页（文本抽查首/中/末页，几何全查）
    path, r = V("long_doc_120p.pdf")
    chk(len(r.pages) == 120, "long_doc_120p 页数 != 120")
    for i, pg in enumerate(r.pages):
        if i not in (0, 59, 118):
            inf = page_verify_info(pg, deep_text=False)
        else:
            inf = page_verify_info(pg, deep_text=True)
            chk(inf["has_text"] and inf["has_cjk_text"] and inf["text_len"] > 40, f"long_doc p{i+1} 文本异常")
        chk(inf["rotation"] == 0 and box_eq(inf["mediabox"], A4_BOX), f"long_doc p{i+1} 几何异常")
    chk(os.path.getsize(path) < MAX_CORPUS_BYTES, "long_doc_120p.pdf 超过 500KB")
    results["long_doc_120p.pdf"] = "120 页 A4，页码唯一，首/中/末页文本抽查通过，<500KB"

    # 12) oversized_page
    path, r = V("oversized_page.pdf")
    chk(len(r.pages) == 1, "oversized_page 页数 != 1")
    inf = page_verify_info(r.pages[0])
    chk(box_eq(inf["mediabox"], [0, 0, 2000, 2800], tol=0.5), "oversized_page MediaBox != 2000×2800")
    chk(inf["has_text"], "oversized_page 应含文本")
    results["oversized_page.pdf"] = "1 页，MediaBox 2000×2800 pt（远超 A4）"

    # 13/14) 坏文件：不应被 pypdf 正常打开（行为单独探测并记录）
    for name in ("damaged_truncated.pdf", "damaged_garbage.pdf"):
        opened = None
        try:
            rr = PdfReader(os.path.join(CORPUS_DIR, name))
            opened = len(rr.pages)
        except Exception as e:
            opened = f"{type(e).__name__}"
        results[name] = f"坏文件（预期 error），pypdf 打开行为：{opened}"
    chk(os.path.getsize(os.path.join(CORPUS_DIR, "damaged_truncated.pdf")) > 0, "damaged_truncated 为空")
    chk(os.path.getsize(os.path.join(CORPUS_DIR, "damaged_garbage.pdf")) == 512 * 1024, "damaged_garbage != 512KB")

    # 15) mixed_features：4 页各含一种特征
    path, r = V("mixed_features.pdf")
    chk(len(r.pages) == 4, "mixed_features 页数 != 4")
    infos = [page_verify_info(pg) for pg in r.pages]
    chk(infos[0]["embedded_fonts"] and infos[0]["has_text"] and infos[0]["has_cjk_text"], "mixed p1 中文嵌入特征缺失")
    chk(not infos[1]["has_text"] and not infos[1]["has_images"], "mixed p2 矢量特征异常")
    chk(infos[2]["alpha_extgstate"], "mixed p3 透明特征缺失")
    chk(infos[3]["rotation"] == 90, "mixed p4 /Rotate != 90")
    for inf in infos:
        chk(box_eq(inf["mediabox"], A4_BOX), "mixed_features 各页应为 A4")
    chk(os.path.getsize(path) < MAX_CORPUS_BYTES, "mixed_features.pdf 超过 500KB")
    results["mixed_features.pdf"] = "4 页 A4：中文/矢量/透明/旋转90，<500KB"

    return results


# ---------------------------------------------------------------- 渲染与网格
def poppler_render(pdf_path, page_no, out_dir):
    """poppler 参照渲染：固定宽 640px（-cropbox/-scale-dimension-before-rotation 对齐 PDFium 语义）"""
    stem = os.path.splitext(os.path.basename(pdf_path))[0]
    prefix = os.path.join(out_dir, f"{stem}_p{page_no}")
    for f in glob.glob(prefix + "-*"):
        os.remove(f)
    cmd = ["pdftoppm", "-png", "-r", "96",
           "-f", str(page_no), "-l", str(page_no),
           "-scale-to-x", str(RASTER_W), "-scale-to-y", "-1",
           "-cropbox", "-scale-dimension-before-rotation",
           pdf_path, prefix]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    outs = sorted(glob.glob(prefix + "-*.png"))
    if not outs:
        raise RuntimeError(f"pdftoppm 未产出 PNG：{os.path.basename(pdf_path)} p{page_no} rc={p.returncode} {p.stderr[-300:]}")
    old = outs[0]
    with Image.open(old) as im:
        w, h = im.size
    final = os.path.join(out_dir, f"{stem}_p{page_no}_{w}x{h}.png")
    os.replace(old, final)
    return final, w, h


def pdfium_render_gray(pdf_path, page_idx):
    """pypdfium2（build 8149，与 Rust 同引擎）渲染：目标宽 640px，返回灰度图与原始位图尺寸"""
    doc = pdfium.PdfDocument(pdf_path)
    try:
        page = doc[page_idx]
        scale = RASTER_W / page.get_width()      # get_width 已应用旋转与 CropBox
        img = page.render(scale=scale).to_pil()
        if abs(img.size[0] - RASTER_W) > 1:
            # 用位图实测自证并纠正 scale（防取整偏差）
            scale2 = scale * RASTER_W / img.size[0]
            img = page.render(scale=scale2).to_pil()
        native = img.size
        return img.convert("L"), native
    finally:
        doc.close()


def center_fit(img, tw, th):
    """高度差 1-2px 时的中心对齐裁剪/补白（避免重采样）"""
    h = img.size[1]
    if h >= th:
        y0 = (h - th) // 2
        return img.crop((0, y0, tw, y0 + th))
    out = Image.new("L", (tw, th), 255)
    out.paste(img, (0, (th - h) // 2))
    return out


def normalize_gray(img_l, tw, th):
    """统一到 (tw, th)：±2px 高度差走中心裁/补，其余 LANCZOS 重采样"""
    if img_l.size[0] != tw:
        nh = max(1, round(img_l.size[1] * tw / img_l.size[0]))
        img_l = img_l.resize((tw, nh), Image.LANCZOS)
    if img_l.size[1] == th:
        return img_l
    if abs(img_l.size[1] - th) <= 2:
        return center_fit(img_l, tw, th)
    return img_l.resize((tw, th), Image.LANCZOS)


def grids_from_gray(img_l, G=GRID_N, ink_t=INK_T):
    """24×24 网格：grid_mean 灰度均值（白=1.0）/ grid_ink 墨迹覆盖率（<ink_t 判墨）"""
    arr = np.asarray(img_l, dtype=np.uint8)
    H, W = arr.shape
    xs = [i * W // G for i in range(G + 1)]
    ys = [j * H // G for j in range(G + 1)]
    mean = np.zeros((G, G), dtype=np.float64)
    ink = np.zeros((G, G), dtype=np.float64)
    for j in range(G):
        for i in range(G):
            cell = arr[ys[j]:ys[j + 1], xs[i]:xs[i + 1]]
            if cell.size == 0:
                continue
            mean[j, i] = cell.mean() / 255.0
            ink[j, i] = float((cell < ink_t).mean())
    return mean, ink


def r4(a):
    return np.round(a, 4).tolist()


def cell_stats(pool):
    if len(pool) == 0:
        return {"count": 0, "avg": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0}
    a = np.asarray(pool, dtype=np.float64)
    return {"count": int(a.size),
            "avg": round(float(a.mean()), 5),
            "p95": round(float(np.percentile(a, 95)), 5),
            "p99": round(float(np.percentile(a, 99)), 5),
            "max": round(float(a.max()), 5)}


def recommend_tol(pool):
    """全语料单一容差：P95 × 1.5，但不低于观测最大值 × 1.05（向上取到 3 位小数）
    （纯 P95×1.5 更严但会让个别格失败，见报告中的分层容差）"""
    if len(pool) == 0:
        return 0.0
    p95 = float(np.percentile(pool, 95))
    mx = float(np.max(pool))
    v = max(p95 * 1.5, mx * 1.05)
    return math.ceil(v * 1000) / 1000


def page_tol(observed_max, floor, margin=1.3):
    """单页容差：观测最大逐格差 × 1.3，向上取 0.005 步长，不低于 floor"""
    v = max(observed_max * margin, floor)
    return math.ceil(v / 0.005) * 0.005


def poppler_version():
    p = subprocess.run(["pdftoppm", "-v"], capture_output=True, text=True)
    m = re.search(r"version\s+([\d.]+)", p.stderr)
    return m.group(1) if m else p.stderr.strip()


# ---------------------------------------------------------------- 坏文件行为探测
def probe_damaged(path):
    """坏文件在三方（pypdf / poppler / pdfium）下的行为记录；pdfium 用子进程隔离防崩溃"""
    name = os.path.basename(path)
    res = {"expected": "error", "bytes": os.path.getsize(path)}

    # pypdf
    try:
        rr = PdfReader(path)
        sample = ""
        try:
            sample = (rr.pages[0].extract_text() or "")[:30]
        except Exception:
            pass
        res["pypdf"] = {"openable": True, "page_count": len(rr.pages), "text_sample": sample}
    except Exception as e:
        res["pypdf"] = {"openable": False, "exception": f"{type(e).__name__}: {str(e)[:200]}"}

    # poppler
    prefix = os.path.join(RENDER_DIR, "_dmg_" + os.path.splitext(name)[0])
    for f in glob.glob(prefix + "-*"):
        os.remove(f)
    try:
        p = subprocess.run(["pdftoppm", "-png", "-r", "96", "-f", "1", "-l", "1",
                            "-scale-to-x", str(RASTER_W), "-scale-to-y", "-1",
                            "-cropbox", "-scale-dimension-before-rotation",
                            path, prefix],
                           capture_output=True, text=True, timeout=90)
        pngs = glob.glob(prefix + "-*.png")
        res["poppler"] = {"returncode": p.returncode,
                          "stderr_head": p.stderr.strip()[:220],
                          "png_produced": bool(pngs)}
        if pngs:
            with Image.open(pngs[0]) as im:
                res["poppler"]["png_size"] = list(im.size)
    except subprocess.TimeoutExpired:
        res["poppler"] = {"returncode": "timeout"}

    # pdfium（子进程隔离）
    code = (
        "import sys, pypdfium2 as p\n"
        "d = p.PdfDocument(sys.argv[1])\n"
        "print('pages', len(d))\n"
        "pg = d[0]\n"
        "bm = pg.render(scale=640/max(pg.get_width(),1))\n"
        "print('bitmap', bm.width, bm.height)\n"
    )
    try:
        p2 = subprocess.run([sys.executable, "-c", code, path],
                            capture_output=True, text=True, timeout=60)
        res["pdfium"] = {"returncode": p2.returncode,
                         "stdout": p2.stdout.strip()[:140],
                         "stderr": p2.stderr.strip()[-220:]}
    except subprocess.TimeoutExpired:
        res["pdfium"] = {"returncode": "timeout"}
    return res


# ---------------------------------------------------------------- 主流程
def main():
    ensure_dirs()
    clean_render_dir()
    ensure_fonts()
    print(f"pdfium build = {PDFIUM_BUILD}（期望 8149）；poppler = {poppler_version()}")

    # ========== 1. 生成 15 个语料 ==========
    print("\n[1/5] 生成语料 PDF ...")
    zh_path = os.path.join(CORPUS_DIR, "zh_embedded_font.pdf")
    gen_zh_embedded_font(zh_path)
    gen_zh_missing_font(os.path.join(CORPUS_DIR, "zh_missing_font.pdf"))
    gen_scanned_page(os.path.join(CORPUS_DIR, "scanned_page.pdf"))
    gen_vector_graphics(os.path.join(CORPUS_DIR, "vector_graphics.pdf"))
    gen_transparency_clip(os.path.join(CORPUS_DIR, "transparency_clip.pdf"))
    gen_rotated_90(os.path.join(CORPUS_DIR, "rotated_90.pdf"))
    gen_rotated_180(os.path.join(CORPUS_DIR, "rotated_180.pdf"))
    gen_landscape_mixed(os.path.join(CORPUS_DIR, "landscape_mixed.pdf"))
    gen_cropbox_offset(os.path.join(CORPUS_DIR, "cropbox_offset.pdf"))
    gen_cropbox_inset(os.path.join(CORPUS_DIR, "cropbox_inset.pdf"))
    gen_long_doc_120p(os.path.join(CORPUS_DIR, "long_doc_120p.pdf"))
    gen_oversized_page(os.path.join(CORPUS_DIR, "oversized_page.pdf"))
    gen_damaged_truncated(os.path.join(CORPUS_DIR, "damaged_truncated.pdf"), zh_path)
    gen_damaged_garbage(os.path.join(CORPUS_DIR, "damaged_garbage.pdf"))
    gen_mixed_features(os.path.join(CORPUS_DIR, "mixed_features.pdf"))

    # ========== 2. pypdf 独立验证 ==========
    print("[2/5] pypdf 独立验证 ...")
    verify_results = verify_corpus()
    print("      全部 15 个文件验证通过 ✓")

    # ========== 3/4. poppler 参照渲染 + pypdfium2 跨引擎校准 ==========
    print("[3/5] poppler 参照渲染（宽 640px）+ [4/5] pypdfium2 同页渲染校准 ...")
    manifest = {
        "poppler_version": poppler_version(),
        "pdfium_build": PDFIUM_BUILD,
        "raster_width_px": RASTER_W,
        "grid": GRID_N,
        "ink_threshold": INK_T,
        "grid_layout": "row-major [row][col]，row 0 = 渲染页顶部",
        "poppler_command": ("pdftoppm -png -r 96 -f N -l N -scale-to-x 640 -scale-to-y -1 "
                            "-cropbox -scale-dimension-before-rotation <pdf> <prefix>"),
        "files": {},
    }
    calib_pages = []
    geom_anomalies = []
    pool_mean_all, pool_ink_all = [], []
    pool_mean_excl, pool_ink_excl = [], []
    page_rows = []   # 终端汇总表

    for name, pages_1b in PROBE_PAGES.items():
        path = os.path.join(CORPUS_DIR, name)
        reader = PdfReader(path)
        fentry = {"bytes": os.path.getsize(path), "page_count": len(reader.pages), "pages": {}}
        for pno in pages_1b:
            idx = pno - 1
            pg = reader.pages[idx]
            w_pt, h_pt, rot = render_geometry_pt(pg)
            H_ref = round(RASTER_W * h_pt / w_pt)
            inf = page_verify_info(pg)

            # ---- poppler 参照 ----
            png_path, wp, hp = poppler_render(path, pno, RENDER_DIR)
            if abs(wp - RASTER_W) > 2:
                raise RuntimeError(f"{name} p{pno} poppler 宽度 {wp} != 640")
            img_p = Image.open(png_path).convert("L")
            mean_p, ink_p = grids_from_gray(normalize_gray(img_p, RASTER_W, H_ref))

            # ---- pypdfium2 校准 ----
            rec = {"file": name, "page_1based": pno}
            try:
                img_f, (wf, hf) = pdfium_render_gray(path, idx)
                mean_f, ink_f = grids_from_gray(normalize_gray(img_f, RASTER_W, H_ref))
                d_mean = np.abs(mean_f - mean_p)
                d_ink = np.abs(ink_f - ink_p)
                rec.update({
                    "expected_px": [RASTER_W, H_ref],
                    "poppler_native": [wp, hp],
                    "pdfium_native": [wf, hf],
                    "poppler_h_minus_ref": hp - H_ref,
                    "pdfium_h_minus_ref": hf - H_ref,
                    "mean_max_cell": round(float(d_mean.max()), 5),
                    "mean_avg_cell": round(float(d_mean.mean()), 5),
                    "ink_max_cell": round(float(d_ink.max()), 5),
                    "ink_avg_cell": round(float(d_ink.mean()), 5),
                })
                # 单页推荐容差（写入 manifest，供 Rust 分页断言）：观测最大 × 1.3，带下限
                rec["tol_mean_max_cell"] = page_tol(rec["mean_max_cell"], floor=0.02)
                rec["tol_ink_max_cell"] = page_tol(rec["ink_max_cell"], floor=0.04)
                pool_mean_all.extend(d_mean.ravel().tolist())
                pool_ink_all.extend(d_ink.ravel().tolist())
                if name != "zh_missing_font.pdf":
                    pool_mean_excl.extend(d_mean.ravel().tolist())
                    pool_ink_excl.extend(d_ink.ravel().tolist())
                # 几何断言：引擎位图纵横比 vs pypdf 基准
                ref_ar = h_pt / w_pt
                for eng, (ew, eh) in (("poppler", (wp, hp)), ("pdfium", (wf, hf))):
                    dev = abs((eh / ew) / ref_ar - 1) * 100
                    rec[f"{eng}_aspect_dev_pct"] = round(dev, 3)
                    if dev > GEOM_ANOMALY_PCT:
                        geom_anomalies.append({
                            "file": name, "page_1based": pno, "engine": eng,
                            "pypdf_render_w_pt": round(w_pt, 2), "pypdf_render_h_pt": round(h_pt, 2),
                            "bitmap_px": [ew, eh], "aspect_dev_pct": round(dev, 3),
                        })
                page_rows.append((name, pno, f"{wp}x{hp}", f"{wf}x{hf}", H_ref,
                                  rec["mean_max_cell"], rec["mean_avg_cell"],
                                  rec["ink_max_cell"], rec["ink_avg_cell"]))
            except Exception as e:
                rec["error"] = f"{type(e).__name__}: {e}"
                page_rows.append((name, pno, f"{wp}x{hp}", "ERR", H_ref, -1, -1, -1, -1))
            calib_pages.append(rec)

            # ---- manifest 页条目 ----
            fentry["pages"][str(idx)] = {
                "pypdf": {
                    "width_pt": inf["mediabox"][2] - inf["mediabox"][0],
                    "height_pt": inf["mediabox"][3] - inf["mediabox"][1],
                    "rotation": inf["rotation"],
                    "mediabox": inf["mediabox"],
                    "cropbox": inf["cropbox"],
                    "render_w_pt": round(w_pt, 2),
                    "render_h_pt": round(h_pt, 2),
                    "embedded_fonts": inf["embedded_fonts"],
                    "has_text": inf["has_text"],
                    "has_images": inf["has_images"],
                },
                "ref_png": os.path.basename(png_path),
                "tol_mean_max_cell": rec.get("tol_mean_max_cell"),
                "tol_ink_max_cell": rec.get("tol_ink_max_cell"),
                "grid_mean": r4(mean_p),
                "grid_ink": r4(ink_p),
            }
        manifest["files"][name] = fentry

    # 坏文件条目（预期 error + 三方行为）
    for name in ("damaged_truncated.pdf", "damaged_garbage.pdf"):
        manifest["files"][name] = probe_damaged(os.path.join(CORPUS_DIR, name))

    # ---- 校准汇总 ----
    st_mean_all, st_ink_all = cell_stats(pool_mean_all), cell_stats(pool_ink_all)
    st_mean_ex, st_ink_ex = cell_stats(pool_mean_excl), cell_stats(pool_ink_excl)
    tol_mean = recommend_tol(pool_mean_excl)
    tol_ink = recommend_tol(pool_ink_excl)
    outlier_note = None
    if st_mean_all["max"] > max(st_mean_ex["max"] * 3, 0.15) or st_ink_all["max"] > max(st_ink_ex["max"] * 3, 0.15):
        outlier_note = ("zh_missing_font.pdf 的字体回退在两引擎间不可控，建议 Rust 断言对该文件单独放宽 "
                        f"（mean ≤ {recommend_tol(pool_mean_all):.3f} / ink ≤ {recommend_tol(pool_ink_all):.3f}）"
                        "或标记为回退渲染样本。")
    calibration = {
        "poppler_version": poppler_version(),
        "pdfium_build": PDFIUM_BUILD,
        "raster_width_px": RASTER_W, "grid": GRID_N, "ink_threshold": INK_T,
        "poppler_command": manifest["poppler_command"],
        "pages": calib_pages,
        "corpus_summary": {
            "all_files": {"mean_cell_diff": st_mean_all, "ink_cell_diff": st_ink_all},
            "excluding_zh_missing_font": {"mean_cell_diff": st_mean_ex, "ink_cell_diff": st_ink_ex},
        },
        "recommended_tolerance": {
            # 全语料单一容差（能过全部探针页，但被巨型数字页抬高）
            "grid_mean_cell_max": tol_mean,
            "grid_ink_cell_max": tol_ink,
            # 任务建议公式（P95 × 1.5）：更严，但会让 base-14 代换页的个别格失败
            "p95_formula": {
                "grid_mean_cell_max": math.ceil(float(np.percentile(pool_mean_excl, 95)) * 1.5 * 1000) / 1000,
                "grid_ink_cell_max": math.ceil(float(np.percentile(pool_ink_excl, 95)) * 1.5 * 1000) / 1000,
            },
            "per_page": "manifest 每页条目带 tol_mean_max_cell / tol_ink_max_cell（观测最大 × 1.3），建议 Rust 分页断言用该值",
            "note": ("全语料单一容差基于剔除 zh_missing_font 后的 P95×1.5 与观测最大×1.05 的较大者；"
                      "巨型 Helvetica 数字页（两引擎 base-14 代换字体不同）与 0.7pt 发丝线（poppler 实心 1px，"
                      "pdfium 抗锯齿）把全语料 max 抬高，分层断言更灵敏。"),
        },
        "geometry_anomalies": geom_anomalies,
        "outlier_note": outlier_note,
    }

    # ========== 5. 落盘 ==========
    print("[5/5] 写 reference_manifest.json / calibration_report.json ...")
    mpath = os.path.join(CORPUS_DIR, "reference_manifest.json")
    with open(mpath, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, separators=(",", ":"))
    cpath = os.path.join(RENDER_DIR, "calibration_report.json")
    with open(cpath, "w", encoding="utf-8") as f:
        json.dump(calibration, f, ensure_ascii=False, indent=1)

    # ---- 终端汇总 ----
    print("\n=== 文件清单 ===")
    for name in sorted(os.listdir(CORPUS_DIR)):
        if not name.endswith(".pdf"):
            continue
        p = os.path.join(CORPUS_DIR, name)
        n = manifest["files"].get(name, {}).get("page_count", "?")
        print(f"  {name:<26} {os.path.getsize(p):>9,} B  {n} 页  {verify_results.get(name, '')}")

    print("\n=== 探针页校准（poppler vs pdfium，宽 640） ===")
    print(f"  {'file':<24} {'pg':>3} {'poppler':>10} {'pdfium':>10} {'Href':>5} {'meanMax':>8} {'meanAvg':>8} {'inkMax':>8} {'inkAvg':>8}")
    for row in page_rows:
        print(f"  {row[0]:<24} {row[1]:>3} {row[2]:>10} {row[3]:>10} {row[4]:>5} {row[5]:>8} {row[6]:>8} {row[7]:>8} {row[8]:>8}")

    print("\n=== 语料级逐格差统计 ===")
    print(f"  全部文件      mean: {st_mean_all}  ")
    print(f"  全部文件       ink: {st_ink_all}")
    print(f"  剔除缺字体    mean: {st_mean_ex}")
    print(f"  剔除缺字体     ink: {st_ink_ex}")
    print(f"  推荐容差（全语料单一，能过全部页）：grid_mean ≤ {tol_mean} / grid_ink ≤ {tol_ink}")
    print(f"  推荐容差（P95×1.5 公式，更严）：grid_mean ≤ {calibration['recommended_tolerance']['p95_formula']['grid_mean_cell_max']}"
          f" / grid_ink ≤ {calibration['recommended_tolerance']['p95_formula']['grid_ink_cell_max']}")
    print("  分页容差已写入 manifest 每页条目（tol_mean_max_cell / tol_ink_max_cell，观测最大×1.3）")
    if outlier_note:
        print("  " + outlier_note)
    print(f"\n  几何异常（纵横比偏差 > {GEOM_ANOMALY_PCT}%）：{len(geom_anomalies)} 条")
    for a in geom_anomalies:
        print("   ", a)

    print("\n=== 坏文件行为 ===")
    for name in ("damaged_truncated.pdf", "damaged_garbage.pdf"):
        print(f"  {name}: {json.dumps(manifest['files'][name], ensure_ascii=False)[:400]}")

    print(f"\nmanifest  → {mpath}")
    print(f"calibration → {cpath}")
    print("DONE")


if __name__ == "__main__":
    main()
