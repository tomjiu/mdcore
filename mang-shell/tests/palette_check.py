#!/usr/bin/env python3
"""#47 批次 9（A4 命令面板开/关）像素断言——纯 stdlib。

用法:
    python3 palette_check.py pal-a.ppm pal-b.ppm pal-c.ppm

输入: --smoke-palette 三帧（A=基态 / B=Ctrl+Shift+P 开面板 / C=Esc 后）。

断言语义:
    PA1 面板打开像素证据——B vs A 面板区变化像素占比 ≥ 下限
       （palette 覆盖盒 + 段标题/条目列表真实渲染）。
    PA2 Esc 关净——C vs A 面板区变化像素占比 ≤ 上限（插入符闪烁
       容差——面板在编辑区内，插入符少量像素为正常残留）。
    PA3 三帧窗口原点一致。
    注: 面板开关是编辑器内联状态翻转（PaletteToggle/Esc 都是键
    分支），无独立证据行面——像素双态即证据（诚实口径；键派发链
    由 keymap E0 表 + ui_wiring_faces 面部2 断言背书）。

面板区（窗口内容系）: x∈[380,930], y∈[80,520]——palette 覆盖盒在
编辑区（x∈[312,1000]）内居中偏上（首跑实测标定）。
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from theme_visual_check import read_p6, detect_window  # noqa: E402

# ── 阈值（2026-11-10 实测标定，见 PR 验证记录）──
OPEN_CHANGED_MIN = 0.02
CLOSED_CHANGED_MAX = 0.005
CONTRAST = 20
PALETTE_X = (380, 930)
PALETTE_Y = (80, 520)


def rgb_at(px, w, x, y):
    o = (y * w + x) * 3
    return px[o], px[o + 1], px[o + 2]


def region_changed_frac(pa, pb, w, x0, x1, y0, y1):
    total = changed = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            ra, ga, ba = rgb_at(pa, w, x, y)
            rb, gb, bb = rgb_at(pb, w, x, y)
            total += 1
            if abs(ra - rb) + abs(ga - gb) + abs(ba - bb) >= CONTRAST:
                changed += 1
    return changed / total


def main():
    if len(sys.argv) != 4:
        print(__doc__)
        return 64
    a_path, b_path, c_path = sys.argv[1:4]
    ap, w, h = read_p6(a_path)
    bp, _, _ = read_p6(b_path)
    cp, _, _ = read_p6(c_path)
    origins = [detect_window(p, w, h) for p in (ap, bp, cp)]
    x0, y0 = origins[0][0], origins[0][1]

    lines, fails = [], []

    def check(name, ok, detail):
        tag = "PASS" if ok else "FAIL"
        lines.append(f"[{tag}] {name}: {detail}")
        if not ok:
            fails.append(name)

    same = all(abs(o[0] - x0) <= 2 and abs(o[1] - y0) <= 2 for o in origins)
    check("PA3-三帧窗口原点一致", same, " / ".join(f"({o[0]},{o[1]})" for o in origins))

    px0, px1 = x0 + PALETTE_X[0], x0 + PALETTE_X[1]
    py0, py1 = y0 + PALETTE_Y[0], y0 + PALETTE_Y[1]

    frac_open = region_changed_frac(ap, bp, w, px0, px1, py0, py1)
    check("PA1-开面板-变化", frac_open >= OPEN_CHANGED_MIN,
          f"B-A 变化像素 {frac_open*100:.2f}% ≥ {OPEN_CHANGED_MIN*100:.1f}%")

    frac_closed = region_changed_frac(ap, cp, w, px0, px1, py0, py1)
    check("PA2-Esc关净-变化占比", frac_closed <= CLOSED_CHANGED_MAX,
          f"C-A 变化像素 {frac_closed*100:.2f}% ≤ {CLOSED_CHANGED_MAX*100:.1f}%（插入符容差）")

    for ln in lines:
        print(ln)
    tag = "OK" if not fails else f"FAIL: {fails}"
    print(f"palette-check: {len(lines)} 项断言，{tag}（{a_path.rsplit('/',1)[-1]} 系 {w}x{h}）")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
