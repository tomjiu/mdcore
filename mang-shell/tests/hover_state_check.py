#!/usr/bin/env python3
"""#47 批次 5（E-2）hover/选中态像素断言——纯 stdlib。

用法:
    python3 hover_state_check.py hover-light-a.ppm hover-light-b.ppm hover-light-c.ppm
    python3 hover_state_check.py hover-dark-{a,b,c}.ppm

输入: --smoke-hover-state 三帧（A=无悬停基态 / B=悬停 git 钮 / C=点击后
      git 钮选中 + 视图切换），Xvfb 1600×1000 全屏帧，窗口原点自动探测。

断言语义（诚实边界）:
    H1 hover 底色出现——git 钮盒 B 帧 vs A 帧：均亮度朝主题 hover 方向
       偏移（light: 变暗 panel 242→hover ~227；dark: 变亮 panel ~33→
       hover ~45），|Δ| ≥ 阈值。
    H2 选中态——git 钮图标区 C vs B：点击后鼠标仍悬停钮上，hover 灰底
       覆盖选中蓝底（样式叠加序：hover 后于 selected 生效），选中证据
       在图标 Color::Selected 变蓝（图标 16×16 居中矩形蓝度增量）。
    H3 文件树钮取消选中——文件树钮盒 C vs A：A 帧它是选中蓝底（启动
       默认 left_open && !git_view），点击 git 后失去，蓝度下降 ≥ 阈值。
    H4 视图切换——左面板区 C vs A 的变化像素占比 ≥ 下限（文件树→git
       面板：仅表头与行内容变化，整面 MAD 对少量行不敏感（实测 1.4），
       改尺度无关的「变化像素占比」）。
    H5 三帧窗口原点一致（截帧时间稳定性）。
    焦点环/禁用态：默认烟雾会话无确定性禁用控件（生产面 grep 实证），
    焦点环渲染依赖键盘 focus_visible 语义——两者登记待确定性夹具后
    随批补，完整四态人工观感归 #48 真机。

按钮盒坐标（窗口内容系，activity_bar.rs 同源）: BAR_PADDING 8 +
36×36 + GAP 4：文件树 [44,80]、git [84,120]。
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from theme_visual_check import read_p6, detect_window, region_stats  # noqa: E402

# ── 阈值（2026-11-10 实测标定——首跑后回填，见 PR 验证记录）──
HOVER_DELTA_MIN = 4        # git 钮盒 B-A 均亮度 |Δ| 下限
SELECT_BLUENESS_MIN = 6    # git 钮图标区 C-B 蓝度增量下限（实测 +13.8：
                           # hover 覆盖选中底——选中证据在图标 Color::Selected 变蓝）
DESELECT_BLUENESS_MIN = 3  # 文件树钮盒 C-A 蓝度减量下限
PANEL_CHANGED_MIN = 0.005  # 左面板区 C-A 变化像素占比下限（对比阈 20）
PANEL_CONTRAST = 20        # 「变化像素」判定亮度差

TITLE_BAR_H = 36
BAR_PAD = 8
BTN = 36
FILE_TREE_Y = (TITLE_BAR_H + BAR_PAD, TITLE_BAR_H + BAR_PAD + BTN)        # 44..80
GIT_Y = (FILE_TREE_Y[0] + BTN + 4, FILE_TREE_Y[1] + BTN + 4)               # 84..120
ACTIVITY_X = (0, 52)
LEFT_PANEL_X = (52, 52 + 260)


def rgb_at(px, w, x, y):
    o = (y * w + x) * 3
    return px[o], px[o + 1], px[o + 2]


def box_means(px, w, x0, x1, y0, y1):
    """盒内 R/G/B 均值 + 亮度均值（全采样，36px 盒小）。"""
    n = r = g = b = 0
    for y in range(y0, y1):
        for x in range(x0, x1):
            rr, gg, bb = rgb_at(px, w, x, y)
            r += rr
            g += gg
            b += bb
            n += 1
    return r / n, g / n, b / n


def main():
    if len(sys.argv) != 4:
        print(__doc__)
        return 64
    a_path, b_path, c_path = sys.argv[1:4]
    ap, w, h = read_p6(a_path)
    bp, bw, bh = read_p6(b_path)
    cp, cw, ch = read_p6(c_path)
    if not (w == bw == cw and h == bh == ch):
        raise SystemExit(f"三帧尺寸不一致: {w}x{h} / {bw}x{bh} / {cw}x{ch}")

    origins = [detect_window(p, w, h) for p in (ap, bp, cp)]
    lines, fails = [], []

    def check(name, ok, detail):
        tag = "PASS" if ok else "FAIL"
        lines.append(f"[{tag}] {name}: {detail}")
        if not ok:
            fails.append(name)

    # H5 三帧原点一致
    o0 = origins[0]
    same = all(abs(o[0] - o0[0]) <= 2 and abs(o[1] - o0[1]) <= 2 for o in origins)
    check("H5-三帧窗口原点一致", same,
          " / ".join(f"({o[0]},{o[1]})" for o in origins))
    x0, y0 = o0[0], o0[1]

    def box(px, ys):
        return box_means(px, w, x0 + 8, x0 + 44, y0 + ys[0], y0 + ys[1])

    # H1 hover 底色（git 钮盒亮度偏移）
    la = box(ap, GIT_Y)
    lb = box(bp, GIT_Y)
    lum = lambda t: 0.299 * t[0] + 0.587 * t[1] + 0.114 * t[2]
    d_hover = lum(lb) - lum(la)
    check("H1-hover-底色偏移", abs(d_hover) >= HOVER_DELTA_MIN,
          f"B-A 亮度 Δ{d_hover:+.1f}（|Δ| ≥ {HOVER_DELTA_MIN}）")

    # H2 选中态（图标区蓝度——点击后鼠标仍悬停钮上，hover 灰底覆盖
    # 选中蓝底（样式叠加序：hover 后于 selected 生效），选中证据在
    # 图标 Color::Selected 变蓝：图标 16×16 居中矩形蓝度增量）
    blue = lambda t: t[2] - (t[0] + t[1]) / 2
    lb_icon = box_means(bp, w, x0 + 18, x0 + 34, y0 + GIT_Y[0] + 10, y0 + GIT_Y[0] + 26)
    lc_icon = box_means(cp, w, x0 + 18, x0 + 34, y0 + GIT_Y[0] + 10, y0 + GIT_Y[0] + 26)
    d_sel = blue(lc_icon) - blue(lb_icon)
    check("H2-选中-图标蓝度增量", d_sel >= SELECT_BLUENESS_MIN,
          f"C-B 图标区蓝度 Δ{d_sel:+.2f} ≥ {SELECT_BLUENESS_MIN}")

    # H3 文件树钮取消选中（蓝度减量）
    fa = box(ap, FILE_TREE_Y)
    fc = box(cp, FILE_TREE_Y)
    d_desel = blue(fc) - blue(fa)
    check("H3-取消选中-蓝度减量", d_desel <= -DESELECT_BLUENESS_MIN,
          f"C-A 蓝度 Δ{d_desel:+.2f} ≤ -{DESELECT_BLUENESS_MIN}")

    # H4 左面板视图切换（变化像素占比）
    lp_x0, lp_x1 = x0 + LEFT_PANEL_X[0], x0 + LEFT_PANEL_X[1]
    total = changed = 0
    for y in range(y0 + TITLE_BAR_H, y0 + 780, 2):
        for x in range(lp_x0, lp_x1, 2):
            ra, ga, ba = rgb_at(ap, w, x, y)
            rc, gc, bc = rgb_at(cp, w, x, y)
            da = abs(ra - rc) + abs(ga - gc) + abs(ba - bc)
            total += 1
            if da >= PANEL_CONTRAST:
                changed += 1
    frac = changed / total
    check("H4-视图切换-面板变化占比", frac >= PANEL_CHANGED_MIN,
          f"C-A 变化像素 {frac*100:.2f}% ≥ {PANEL_CHANGED_MIN*100:.1f}%（对比阈 {PANEL_CONTRAST}）")

    for ln in lines:
        print(ln)
    tag = "hover-state " + ("OK" if not fails else f"FAIL: {fails}")
    print(f"hover-state-check: {len(lines)} 项断言，{tag}（{a_path.rsplit('/',1)[-1]} 系 {w}x{h}）")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
