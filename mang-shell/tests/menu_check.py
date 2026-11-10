#!/usr/bin/env python3
"""#47 批次 7（A4 菜单开/关）像素+日志断言——纯 stdlib。

用法:
    python3 menu_check.py menu-a.ppm menu-b.ppm menu-c.ppm menu-app.log

输入: --smoke-menu 三帧（A=基态 / B=点击「文件」开菜单 / C=Esc 后）。

断言语义:
    M1 菜单打开像素证据——B vs A 下拉区变化像素占比 ≥ 下限
       （ui::ContextMenu 浮层 + 菜单项文本真实渲染）。
    M2 Esc 关净——C vs A 下拉区逐字节相等（浮层完全消失，恢复基态）。
    M3 三帧窗口原点一致。
    M4 开/关双证据行——日志含 `menu: open 文件` 与 `menu: close`
       （menu_bar_view 触发器 listener 两分支证据行，批次 7 加）。
    注: 菜单项执行与 Esc 关闭链（依赖 open_menu_bar 两层 on_next_frame
    焦点注入——无头烟雾无输入泵帧不可达）的证据面在头单测
    （menu_bar_lifecycle_tests + ui_wiring_faces 派发断言）；本脚本
    覆盖「点击开 → 点击 toggle 关」的 GUI 级可视面（A4 真实点击）。

下拉区（窗口内容系）: x∈[0,320], y∈[36,420]——标题栏下缘起、首菜单
宽度 + 常规下拉深度（菜单_3 存档与批次 5 帧实测锚定）。
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from theme_visual_check import read_p6, detect_window  # noqa: E402

# ── 阈值（2026-11-10 实测标定，见 PR 验证记录）──
DROPDOWN_CHANGED_MIN = 0.01   # B-A 下拉区变化像素占比下限
CONTRAST = 20                  # 「变化像素」判定亮度差
DROPDOWN_X = (0, 320)
DROPDOWN_Y = (36, 420)


def rgb_at(px, w, x, y):
    o = (y * w + x) * 3
    return px[o], px[o + 1], px[o + 2]


def region_identical(pa, pb, w, x0, x1, y0, y1):
    for y in range(y0, y1):
        for x in range(x0, x1):
            if rgb_at(pa, w, x, y) != rgb_at(pb, w, x, y):
                return False
    return True


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

    # M3 原点一致
    same = all(abs(o[0] - x0) <= 2 and abs(o[1] - y0) <= 2 for o in origins)
    check("M3-三帧窗口原点一致", same, " / ".join(f"({o[0]},{o[1]})" for o in origins))

    dx0, dx1 = x0 + DROPDOWN_X[0], x0 + DROPDOWN_X[1]
    dy0, dy1 = y0 + DROPDOWN_Y[0], y0 + DROPDOWN_Y[1]

    # M1 菜单打开（B vs A 变化占比）
    frac = region_changed_frac(ap, bp, w, dx0, dx1, dy0, dy1)
    check("M1-开菜单-下拉区变化", frac >= DROPDOWN_CHANGED_MIN,
          f"B-A 变化像素 {frac*100:.2f}% ≥ {DROPDOWN_CHANGED_MIN*100:.1f}%（对比阈 {CONTRAST}）")

    # M2 Esc 关净（C vs A 逐字节相等）
    closed = region_identical(ap, cp, w, dx0, dx1, dy0, dy1)
    check("M2-Esc关净-逐字节相等", closed,
          "C 帧与 A 帧下拉区逐字节相等 ✓" if closed else "C ≠ A（菜单未关净/残留浮层）")

    # M4 开/关双证据行
    with open(log_path, encoding="utf-8", errors="replace") as f:
        log = f.read()
    has_open = "menu: open 文件" in log
    has_close = "menu: close" in log
    check("M4-开/关证据行", has_open and has_close,
          f"open={'✓' if has_open else '✗'} / close={'✓' if has_close else '✗'}")

    for ln in lines:
        print(ln)
    tag = "OK" if not fails else f"FAIL: {fails}"
    print(f"menu-check: {len(lines)} 项断言，{tag}（{a_path.rsplit('/',1)[-1]} 系 {w}x{h}）")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
