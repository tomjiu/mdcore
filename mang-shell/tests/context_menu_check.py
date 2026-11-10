#!/usr/bin/env python3
"""#47 批次 8（A4 右键菜单开/关）像素+日志断言——纯 stdlib。

用法:
    python3 context_menu_check.py cm-a.ppm cm-b.ppm cm-c.ppm cm-app.log

输入: --smoke-context-menu 三帧（A=基态 / B=右键编辑区开菜单 / C=点外关后）。

断言语义:
    K1 右键开菜单像素证据——B vs A 菜单区变化像素占比 ≥ 下限
       （ui::ContextMenu 浮层 + 菜单项真实渲染——锚定在右键位 (600,300)）。
    K2 点外关净——C vs A 菜单区变化像素占比 ≤ 上限（浮层消失；右键在
       编辑器留插入符/焦点移动，少量像素残留为正常——占比而非逐字节，
       阈值实测标定）。
    K3 三帧窗口原点一致。
    K4 开菜单证据行——日志含 `context-menu: open`（build_context_menu
       单点证据行，两个 open_* 共用面，批次 8 加）。

菜单区（窗口内容系）: x∈[560,840], y∈[260,620]——右键位 (600,300) 为
锚、常规浮层宽 ~240/深 ~320（实测后标定）。
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from theme_visual_check import read_p6, detect_window  # noqa: E402

# ── 阈值（2026-11-10 实测标定，见 PR 验证记录）──
OPEN_CHANGED_MIN = 0.02     # B-A 菜单区变化占比下限
CLOSED_CHANGED_MAX = 0.005  # C-A 菜单区变化占比上限（插入符残留容差）
CONTRAST = 20
MENU_X = (560, 840)
MENU_Y = (260, 620)


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
    if len(sys.argv) != 5:
        print(__doc__)
        return 64
    a_path, b_path, c_path, log_path = sys.argv[1:5]
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
    check("K3-三帧窗口原点一致", same, " / ".join(f"({o[0]},{o[1]})" for o in origins))

    mx0, mx1 = x0 + MENU_X[0], x0 + MENU_X[1]
    my0, my1 = y0 + MENU_Y[0], y0 + MENU_Y[1]

    frac_open = region_changed_frac(ap, bp, w, mx0, mx1, my0, my1)
    check("K1-右键开菜单-变化", frac_open >= OPEN_CHANGED_MIN,
          f"B-A 变化像素 {frac_open*100:.2f}% ≥ {OPEN_CHANGED_MIN*100:.1f}%")

    frac_closed = region_changed_frac(ap, cp, w, mx0, mx1, my0, my1)
    check("K2-点外关净-变化占比", frac_closed <= CLOSED_CHANGED_MAX,
          f"C-A 变化像素 {frac_closed*100:.2f}% ≤ {CLOSED_CHANGED_MAX*100:.1f}%（插入符容差）")

    with open(log_path, encoding="utf-8", errors="replace") as f:
        log = f.read()
    has_open = "context-menu: open" in log
    check("K4-开菜单证据行", has_open,
          "context-menu: open 在档 ✓" if has_open else "缺 context-menu: open 证据行")

    for ln in lines:
        print(ln)
    tag = "OK" if not fails else f"FAIL: {fails}"
    print(f"context-menu-check: {len(lines)} 项断言，{tag}（{a_path.rsplit('/',1)[-1]} 系 {w}x{h}）")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
