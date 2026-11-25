#!/usr/bin/env python3
"""D1 归因实验（#51 工作线 D）——rotated_90.pdf 参照失败的四端点拆分。

背景（#51 D1 悬案，Lead 2026-11-14 登记）：Windows/poppler 26.09 机器上
engine_content_matches_poppler_reference 的 rotated_90.pdf p0 静态网格差
0.2569 > 容差 0.24；Linux/poppler 25.03 机器上 0.1831 绿。同一 fixture
（sha256 d4b8ef04…）同一 manifest——差异全在渲染端。

本脚本把「渲染端」拆成三个正交成分：
  1. poppler 版本差：pdftoppm 25.03 vs 26.09（同机 Linux，conda 双版本）
  2. PDFium 平台差：PDFium/Windows（Lead 附件 win-p0.png）vs PDFium/Linux
     （bblanchon 160.0.8165，bin/lib——i51_render 例程产物）
  3. 引擎间差：PDFium vs poppler（同机）——PARITY 活参照门的既有绿面

端点：W(win-p0.png 附件) / P160(pdfium160-p0.bgra) / L25 / L26 / S(manifest
静态网格)。两两 24×24 网格 mean 最大逐格差（与 build_corpus.py /
pdf_display_verify.rs 的 PIL L24 公式 + floor 边界 + 均值/255 + 线性重采样
fit 到 H_ref=453 逐位同式）。

用法：python3 d1_attribution.py <evidence_dir>，evidence_dir 含五端点文件
（生成命令见 #51 认领/交付评论）。输出对照矩阵 + 逐格定位 JSON。
"""
import json
import sys
from pathlib import Path

from PIL import Image

GRID_N = 24
INK_THRESHOLD = 200
H_REF = 453  # 640 × 595.28/841.89 = 452.8 → round=453（pypdf 几何基准）
RASTER_W = 640


def pil_luma_rgb(r: int, g: int, b: int) -> int:
    """PIL convert('L') 的 L24 整数公式——pdf_display_verify.rs 同式。"""
    return (r * 19595 + g * 38470 + b * 7471 + 0x8000) >> 16


def luma_from_bgra(bgra: bytes, w: int, h: int) -> list[list[int]]:
    assert len(bgra) == w * h * 4
    return [
        [pil_luma_rgb(bgra[(y * w + x) * 4 + 2], bgra[(y * w + x) * 4 + 1], bgra[(y * w + x) * 4 + 0])
         for x in range(w)]
        for y in range(h)
    ]


def luma_from_png(path: Path) -> list[list[int]]:
    im = Image.open(path).convert("RGB")
    w, h = im.size
    px = im.load()
    return [[pil_luma_rgb(*px[x, y][:3]) for x in range(w)] for y in range(h)]


def fit_height(rows: list[list[int]], target_h: int) -> list[list[int]]:
    """线性重采样（像素中心映射 + 双行线性插值 + round）——测试侧
    fit_height_to_reference 同式。"""
    h = len(rows)
    if h == 0 or h == target_h:
        return rows
    w = len(rows[0])
    out = [[255] * w for _ in range(target_h)]
    for j in range(target_h):
        sy = (j + 0.5) * h / target_h - 0.5
        y0 = max(0, min(int(sy // 1), h - 1))
        y1 = min(y0 + 1, h - 1)
        frac = min(max(sy - y0, 0.0), 1.0)
        for x in range(w):
            v0, v1 = rows[y0][x], rows[y1][x]
            out[j][x] = int(round(v0 + (v1 - v0) * frac))
    return out


def grids(rows: list[list[int]]) -> list[list[float]]:
    """24×24 mean 网格（floor 边界；均值/255 白=1.0）。"""
    h, w = len(rows), len(rows[0])
    xs = [i * w // GRID_N for i in range(GRID_N + 1)]
    ys = [j * h // GRID_N for j in range(GRID_N + 1)]
    mean = [[0.0] * GRID_N for _ in range(GRID_N)]
    for j in range(GRID_N):
        for i in range(GRID_N):
            s = 0
            n = 0
            for y in range(ys[j], ys[j + 1]):
                for x in range(xs[i], xs[i + 1]):
                    s += rows[y][x]
                    n += 1
            if n:
                mean[j][i] = s / (n * 255.0)
    return mean


def grid_worst(a: list[list[float]], b: list[list[float]]):
    """最大逐格差 + 最差格坐标。"""
    worst, pos = 0.0, (0, 0)
    for j in range(len(a)):
        for i in range(len(a[j])):
            d = abs(a[j][i] - b[j][i])
            if d > worst:
                worst, pos = d, (j, i)
    return worst, pos


def main():
    ev = Path(sys.argv[1])
    # 端点装载（p0 主面；p1 备用面不进矩阵）
    win = grids(fit_height(luma_from_png(ev / "win-p0.png"), H_REF))
    p160 = grids(fit_height(luma_from_bgra((ev / "pdfium160-p0.bgra").read_bytes(), RASTER_W, H_REF), H_REF))
    l25 = grids(fit_height(luma_from_png(ev / "pop2503-p0-1.png"), H_REF))
    l26 = grids(fit_height(luma_from_png(ev / "pop2609-p0-1.png"), H_REF))
    man = json.loads((ev / "manifest_excerpt.json").read_text())

    eps = {"W(win-pdfium)": win, "P160(linux-pdfium)": p160, "L25(poppler 25.03)": l25, "L26(poppler 26.09)": l26, "S(静态网格)": man["grid_mean"]}

    names = list(eps)
    print("rotated_90.pdf p0  D1 归因矩阵（24×24 mean 最大逐格差，容差 0.24）")
    print(f"{'':>22}" + "".join(f"{n[:12]:>14}" for n in names))
    matrix = {}
    for a in names:
        row = []
        for b in names:
            if a == b:
                row.append(0.0)
                continue
            d, _ = grid_worst(eps[a], eps[b])
            row.append(d)
        matrix[a] = row
        print(f"{a[:22]:>22}" + "".join(f"{d:>14.4f}" for d in row))

    # 归因成分提炼
    def dd(a, b):
        return matrix[a][names.index(b)]

    print()
    print("成分拆分：")
    comp = {
        "poppler 版本差 (L25 vs L26)": dd("L25(poppler 25.03)", "L26(poppler 26.09)"),
        "引擎间差 Linux (P160 vs L25)": dd("P160(linux-pdfium)", "L25(poppler 25.03)"),
        "引擎间差 Linux (P160 vs L26)": dd("P160(linux-pdfium)", "L26(poppler 26.09)"),
        "PDFium 平台差 (W vs P160)": dd("W(win-pdfium)", "P160(linux-pdfium)"),
        "Windows 静态门路径 (W vs S)": dd("W(win-pdfium)", "S(静态网格)"),
        "Linux 静态门路径 (P160 vs S)": dd("P160(linux-pdfium)", "S(静态网格)"),
        "参照复现性 (L25 vs S)": dd("L25(poppler 25.03)", "S(静态网格)"),
    }
    for k, v in comp.items():
        print(f"  {k:<44}{v:>10.4f}")

    out = {"grid_matrix": {a: dict(zip(names, map(lambda d: round(d, 6), matrix[a]))) for a in names}, "components": {k: round(v, 6) for k, v in comp.items()}}
    (ev / "d1_attribution_result.json").write_text(json.dumps(out, indent=1, ensure_ascii=False))
    print(f"\n结果落盘：{ev / 'd1_attribution_result.json'}")


if __name__ == "__main__":
    main()
