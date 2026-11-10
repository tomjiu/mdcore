#!/usr/bin/env python3
"""#47 批次 4（E-1）主题两态视觉验收——纯 stdlib PPM 像素断言。

用法:
    python3 theme_visual_check.py theme-light.ppm theme-dark.ppm [W] [H]

输入: ffmpeg x11grab 截帧的 P6 PPM（mang-shell 1280×800 窗口全帧）。
判定: 全部断言通过 exit 0，任一失败 exit 1（每条断言打印实测值与阈值，
      阈值出处见各断言注释——按 main 实测值留 margin 标定，不写魔法数）。

布局区域约定（与 mang-shell/src 常量同源，勿漂移）:
    标题栏     y ∈ [0, 36)        title_bar.rs TITLE_BAR_HEIGHT = 36
    活动条     x ∈ [0, 52)        activity_bar.rs 条宽 52
    左面板     x ∈ [52, 312)      panels.rs LEFT_PANEL_WIDTH = 260
    右 dock    x ∈ [W-280, W)     panels.rs RIGHT_PANEL_WIDTH = 280

断言语义（诚实边界）:
    A1 两态底色分向——明态全帧均亮度高 / 暗态低（主题真实作用于像素）。
    A2 切换真实生效——两帧逐像素平均绝对差 ≥ 阈值（非同帧/未切换可检出）。
    A3 侧栏底色随主题——活动条+左面板 / 右 dock 三区域的均亮度与全帧
       同向。若迁移组件把透明底错配（透明按钮缺陷的像素面），侧栏会
       保留对面主题的深/浅底 → 均亮度跑出区间。
    A4 侧栏控件可见性——活动条区域亮度分布存在对比（图标/按钮笔画
       与底色的 spread）。全透明/未渲染的控件会让 spread 塌缩。
    注:「侧栏无透明按钮」的完整人工观感（圆角/焦点环/尺寸统一）仍属
       #48 真机验收；本脚本提供其像素级自动化证据面。
"""
import struct
import sys

# ── 布局常量（与 src 同源）─────────────────────────────────────
TITLE_BAR_H = 36
ACTIVITY_BAR_W = 52
LEFT_PANEL_W = 260
RIGHT_DOCK_W = 280

# ── 断言阈值（2026-11-10 main 3d34cae 实测标定，Xvfb 1600×1000 + 窗口
#    原点探测(100,64)后实测: light 全帧均亮度 244.6, dark 37.4;
#    左侧栏 light 240.8 / dark 37.4; 右 dock light 241.0 / dark 37.3;
#    双帧 MAD 132.8; 活动条墨迹占比 light 1.05% / dark 0.95%）──
LIGHT_FRAME_MIN = 150          # light 全帧均亮度下限（实测 244.6 - 大 margin）
DARK_FRAME_MAX = 90             # dark 全帧均亮度上限（实测 37.4 + margin）
FRAME_DIFF_MIN = 25             # 双帧平均绝对差下限（实测 132.8）
SIDEBAR_LIGHT_MIN = 180         # light 侧栏均亮度下限（实测 ~241）
SIDEBAR_DARK_MAX = 75           # dark 侧栏均亮度上限（实测 ~37）
ICON_SPREAD_MIN = 40            # 活动条「墨迹」与中位底色的对比亮度差下限
INK_FRACTION_MIN = 0.002        # 活动条墨迹占比下限（6 图标×~16px 笔画 ≈1.5%，下限留 7 倍余量）
WIN_CONTENT_W = 1280            # 应用窗口内容宽（main.rs 窗口选项）
WIN_CONTENT_H = 800             # 应用窗口内容高
WIN_TOL = 24                    # 装饰/探测容差（px）


def read_p6(path):
    """读 P6 PPM → (bytes[(y*W+x)*3], W, H)。头两行外允许 # 注释。"""
    with open(path, "rb") as f:
        data = f.read()
    if not data.startswith(b"P6"):
        raise SystemExit(f"{path}: 非 P6 PPM")
    pos = 2
    fields = []
    while len(fields) < 3:
        while pos < len(data) and data[pos : pos + 1].isspace():
            pos += 1
        if data[pos : pos + 1] == b"#":
            while pos < len(data) and data[pos : pos + 1] != b"\n":
                pos += 1
            continue
        start = pos
        while pos < len(data) and not data[pos : pos + 1].isspace():
            pos += 1
        fields.append(int(data[start:pos]))
    pos += 1  # 单空白定界
    w, h, maxv = fields
    if maxv != 255:
        raise SystemExit(f"{path}: maxval {maxv} ≠ 255")
    px = data[pos : pos + w * h * 3]
    if len(px) != w * h * 3:
        raise SystemExit(f"{path}: 像素载荷 {len(px)} ≠ {w*h*3}")
    return px, w, h


def lum_at(px, w, x, y):
    o = (y * w + x) * 3
    r, g, b = px[o], px[o + 1], px[o + 2]
    return 0.299 * r + 0.587 * g + 0.114 * b


def region_stats(px, w, h, x0, x1, y0, y1):
    """区域均亮度 + 亮度分位数（抽样步长控制纯 Python 开销）。"""
    step_x = max(1, (x1 - x0) // 128)
    step_y = max(1, (y1 - y0) // 128)
    vals = []
    for y in range(y0, y1, step_y):
        for x in range(x0, x1, step_x):
            vals.append(lum_at(px, w, x, y))
    vals.sort()
    n = len(vals)
    mean = sum(vals) / n
    median = vals[n // 2]
    hi = vals[int(n * 0.99)]
    return mean, median, hi


def region_ink(px, w, h, x0, x1, y0, y1, thresh):
    """区域「墨迹占比」：与区域中位亮度对比 ≥ thresh 的像素比例。
    图标笔画在 52px 条内仅占 <2% 面积，分位数统计（p99）采不到——
    改占全采样集比例；图标窄笔画 → x 步长取 1 加密。"""
    vals = []
    for y in range(y0, y1, 2):
        for x in range(x0, x1):
            vals.append(lum_at(px, w, x, y))
    if not vals:
        return 0.0, 0.0
    vals.sort()
    median = vals[len(vals) // 2]
    ink = sum(1 for v in vals if abs(v - median) >= thresh) / len(vals)
    return median, ink


def detect_window(px, w, h):
    """探测应用窗口原点与可见尺寸——无 WM 的 Xvfb 下窗口带服务端装饰
    边框，起点不在屏幕原点（实测约 (+104,+56)）。取亮度 > BG_EPS 的
    像素包围盒（深色主题内容 ~34 亮度，仍高于黑屏背景 0，两种主题
    均可探测）。返回 (x0, y0, x1, y1) 内容包围盒。"""
    bg_eps = 10
    xs, ys = [], []
    for y in range(0, h, 4):
        row_hit = False
        for x in range(0, w, 4):
            if lum_at(px, w, x, y) > bg_eps:
                xs.append(x)
                ys.append(y)
                row_hit = True
    if not xs:
        raise SystemExit("帧内未探测到任何窗口内容（全黑？）")
    return min(xs), min(ys), max(xs), max(ys)


def frame_mad(a, b, w, h):
    """两帧平均绝对差（通道合并——亮度差已足够分向判定）。"""
    step = 4  # 每 4 像素抽 1，控制纯 Python 开销（1280×800 → 25.6 万样本）
    total = 0
    n = 0
    for y in range(0, h, step):
        for x in range(0, w, step):
            o = (y * w + x) * 3
            da = lum_at(a, w, x, y) - lum_at(b, w, x, y)
            total += abs(da)
            n += 1
    return total / n


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 64
    light_path, dark_path = sys.argv[1], sys.argv[2]
    lp, w, h = read_p6(light_path)
    dp, dw, dh = read_p6(dark_path)
    if (w, h) != (dw, dh):
        raise SystemExit(f"两帧尺寸不一致: {w}x{h} vs {dw}x{dh}")

    # 窗口原点探测（两帧一致才可信；装饰边框导致内容盒略小于
    # WIN_CONTENT_W/H，容差 WIN_TOL 内认可）。
    lx0, ly0, lx1, ly1 = detect_window(lp, w, h)
    dx0, dy0, dx1, dy1 = detect_window(dp, w, h)
    lines = []
    fails = []

    def check(name, ok, detail):
        tag = "PASS" if ok else "FAIL"
        lines.append(f"[{tag}] {name}: {detail}")
        if not ok:
            fails.append(name)

    check("A0-两帧窗口原点一致",
          abs(lx0 - dx0) <= 2 and abs(ly0 - dy0) <= 2,
          f"light({lx0},{ly0}) vs dark({dx0},{dy0})")
    win_w = lx1 - lx0
    win_h = ly1 - ly0
    check("A0-窗口内容尺寸",
          abs(win_w - WIN_CONTENT_W) <= WIN_TOL and abs(win_h - WIN_CONTENT_H) <= WIN_TOL,
          f"探测 {win_w}x{win_h} vs 预期 {WIN_CONTENT_W}x{WIN_CONTENT_H} (容差 {WIN_TOL})")
    if fails:
        for ln in lines:
            print(ln)
        print(f"theme-visual: 窗口探测失败，后续断言未执行: {fails}")
        return 1

    # A1 两态底色分向（窗口内容盒内，排除黑屏背景）
    lm = region_stats(lp, w, h, lx0, lx1, ly0, ly1)[0]
    dm = region_stats(dp, w, h, dx0, dx1, dy0, dy1)[0]
    check("A1-light-全帧均亮度", lm >= LIGHT_FRAME_MIN,
          f"实测 {lm:.1f} ≥ {LIGHT_FRAME_MIN}")
    check("A1-dark-全帧均亮度", dm <= DARK_FRAME_MAX,
          f"实测 {dm:.1f} ≤ {DARK_FRAME_MAX}")

    # A2 切换真实生效
    mad = frame_mad(lp, dp, w, h)
    check("A2-双帧平均绝对差", mad >= FRAME_DIFF_MIN,
          f"实测 {mad:.1f} ≥ {FRAME_DIFF_MIN}")

    # A3 侧栏底色随主题（活动条+左面板 / 右 dock——窗口原点相对区域）
    y0 = ly0 + TITLE_BAR_H
    left_x = (lx0, lx0 + ACTIVITY_BAR_W + LEFT_PANEL_W)
    right_x = (lx1 - RIGHT_DOCK_W, lx1)
    for name, (x0, x1) in (("左侧栏(活动条+面板)", left_x), ("右dock", right_x)):
        l_mean, _, _ = region_stats(lp, w, h, x0, x1, y0, ly1)
        d_mean, _, _ = region_stats(dp, w, h, dx0 + (x0 - lx0), dx0 + (x1 - lx0), y0, ly1)
        check(f"A3-light-{name}", l_mean >= SIDEBAR_LIGHT_MIN,
              f"均亮度 {l_mean:.1f} ≥ {SIDEBAR_LIGHT_MIN}")
        check(f"A3-dark-{name}", d_mean <= SIDEBAR_DARK_MAX,
              f"均亮度 {d_mean:.1f} ≤ {SIDEBAR_DARK_MAX}")

    # A4 侧栏控件可见性（墨迹占比——图标笔画与底色的对比面积）
    for mode, px, ox in (("light", lp, lx0), ("dark", dp, dx0)):
        median, ink = region_ink(px, w, h, ox, ox + ACTIVITY_BAR_W, y0, ly1, ICON_SPREAD_MIN)
        check(f"A4-{mode}-活动条墨迹占比", ink >= INK_FRACTION_MIN,
              f"中位 {median:.0f}，墨迹 {ink*100:.2f}% ≥ {INK_FRACTION_MIN*100:.1f}%（对比阈 {ICON_SPREAD_MIN}）")

    for ln in lines:
        print(ln)
    if fails:
        print(f"theme-visual: {len(fails)} 项断言失败: {', '.join(fails)}")
        return 1
    print(f"theme-visual: {len(lines)} 项断言全过（{light_path} / {dark_path} {w}x{h}）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
