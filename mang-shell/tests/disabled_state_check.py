#!/usr/bin/env python3
"""#47 批次 6（E-2 禁用态）像素+日志断言——纯 stdlib。

用法:
    python3 disabled_state_check.py disabled a.ppm b.ppm c.ppm app.log
    python3 disabled_state_check.py control  a.ppm b.ppm c.ppm app.log

两种面:
    disabled = 非 git 工作区（GitPanel 构造但 root 非 git 仓——刷新钮应处禁用态）
    control  = 带工作区（git 仓可用——刷新钮正常态，作对照）

断言语义:
    disabled 面:
      X1 刷新钮渲染在场——钮盒内存在墨迹（标签「刷新」笔画）。
      X2 禁用无 hover 响应——B 帧（悬停后）与 A 帧在钮盒内逐字节相等。
      X3 禁用点击无响应——C 帧（点击后）与 B 帧钮盒内逐字节相等，且
         日志 `git: ` 证据行数保持构造期 1 次（不可用文案，点击后不增）。
    control 面:
      X4 hover 有响应——B-A 钮盒均亮度 |Δ| ≥ 阈值（正常 hover 底色）。
      X5 点击有响应——日志 `git: ` 证据行数 ≥ 2（构造 1 次 + 点击刷新 1 次）。
    注: 禁用/对照共享同一旗标时间线（--smoke-disabled-state）。

钮盒坐标（窗口内容系）: x∈[240,290], y∈[84,102]——git 面板头部右侧
（批次 5 C 帧实测标签盒 x∈[249,281] y∈[91,96]，放宽含 px_2 py_1 边距）。
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from theme_visual_check import read_p6, detect_window  # noqa: E402

# ── 阈值（2026-11-10 实测标定，见 PR 验证记录）──
CONTROL_HOVER_DELTA_MIN = 4   # 对照面 B-A 均亮度 |Δ| 下限
REFRESH_BTN_X = (240, 290)     # 钮盒（内容系）
REFRESH_BTN_Y = (84, 102)


def rgb_at(px, w, x, y):
    o = (y * w + x) * 3
    return px[o], px[o + 1], px[o + 2]


def box_stats(px, w, x0, x1, y0, y1):
    """盒内 R/G/B 均值 + 亮度均值 + 与中位差 ≥40 的墨迹占比。"""
    vals = []
    for y in range(y0, y1):
        for x in range(x0, x1):
            r, g, b = rgb_at(px, w, x, y)
            vals.append((r, g, b))
    n = len(vals)
    mr = sum(v[0] for v in vals) / n
    mg = sum(v[1] for v in vals) / n
    mb = sum(v[2] for v in vals) / n
    lums = sorted(0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2] for v in vals)
    median = lums[n // 2]
    ink = sum(1 for l in lums if abs(l - median) >= 40) / n
    lum_mean = sum(lums) / n
    return mr, mg, mb, lum_mean, ink


def box_identical(pa, pb, w, x0, x1, y0, y1):
    """钮盒逐字节相等（禁用 = 零视觉响应）。"""
    for y in range(y0, y1):
        for x in range(x0, x1):
            if rgb_at(pa, w, x, y) != rgb_at(pb, w, x, y):
                return False
    return True


def count_git_evidence(log_path):
    with open(log_path, encoding="utf-8", errors="replace") as f:
        return sum(1 for line in f if line.startswith("git: "))


def main():
    if len(sys.argv) != 6:
        print(__doc__)
        return 64
    face, a_path, b_path, c_path, log_path = sys.argv[1:6]
    ap, w, h = read_p6(a_path)
    bp, _, _ = read_p6(b_path)
    cp, _, _ = read_p6(c_path)
    x0, y0, _, _ = detect_window(ap, w, h)

    bx0, bx1 = x0 + REFRESH_BTN_X[0], x0 + REFRESH_BTN_X[1]
    by0, by1 = y0 + REFRESH_BTN_Y[0], y0 + REFRESH_BTN_Y[1]
    lines, fails = [], []

    def check(name, ok, detail):
        tag = "PASS" if ok else "FAIL"
        lines.append(f"[{tag}] {name}: {detail}")
        if not ok:
            fails.append(name)

    _, _, _, lum_a, ink_a = box_stats(ap, w, bx0, bx1, by0, by1)
    _, _, _, lum_b, ink_b = box_stats(bp, w, bx0, bx1, by0, by1)

    # X1 钮渲染在场（两面都查——禁用态是「灰显」不是「消失」）
    check(f"X1-{face}-刷新钮渲染在场", ink_a >= 0.01,
          f"A 帧钮盒墨迹 {ink_a*100:.2f}% ≥ 1%")

    if face == "disabled":
        same_h = box_identical(ap, bp, w, bx0, bx1, by0, by1)
        check("X2-禁用-hover零响应", same_h, "B 帧与 A 帧钮盒逐字节相等 ✓" if same_h else "B ≠ A（出现 hover 响应——禁用失效）")
        same_c = box_identical(bp, cp, w, bx0, bx1, by0, by1)
        n_git = count_git_evidence(log_path)
        check("X3-禁用-点击零响应", same_c and n_git == 1,
              f"C 帧与 B 帧钮盒逐字节相等={'✓' if same_c else '✗'}，git: 证据行 {n_git} = 1（构造期不可用文案，点击零增量）")
    elif face == "control":
        d = lum_b - lum_a
        check("X4-对照-hover有响应", abs(d) >= CONTROL_HOVER_DELTA_MIN,
              f"B-A 钮盒亮度 Δ{d:+.1f}（|Δ| ≥ {CONTROL_HOVER_DELTA_MIN}）")
        n_git = count_git_evidence(log_path)
        check("X5-对照-点击有响应", n_git >= 2,
              f"git: 证据行 {n_git} ≥ 2（构造 1 + 点击刷新 ≥1）")
    else:
        print(f"未知面: {face}")
        return 64

    for ln in lines:
        print(ln)
    tag = "OK" if not fails else f"FAIL: {fails}"
    print(f"disabled-state-check[{face}]: {len(lines)} 项断言，{tag}（{a_path.rsplit('/',1)[-1]} 系 {w}x{h}）")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
