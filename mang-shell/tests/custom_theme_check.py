#!/usr/bin/env python3
"""#110 G1 批C：自定义主题三帧像素断言（--smoke-custom-theme）。

帧时刻（theme_visual_smoke.sh 批次 10 拍摄）：
  A=5.8s  自定义色应用态（启动 user:g1c-smoke 恢复——background #3366cc 蓝调）
  B=7.6s  dark 切换后（4.2s set_theme_mode(dark)——三态轴真实动作链）
  C=9.4s  dark 稳态

断言（对照 menu_check.py 范式——detect_window 原点系）：
  C1 三帧窗口原点一致（M3 同款）
  C2 自定义色应用（A 帧编辑区亮度——editor 槽生效；色值经 parse_hsla
      灰度近似装载（h=0/s=0 只留 l——G4 登记语义），故亮度判异）
  C3 dark 回退生效（C 帧编辑区近黑——切 dark 后回三态轴）
  C4 亮度差判异（A 帧亮度 - C 帧亮度 ≥ 25——四态→三态真实切换）
  C5 证据行（custom-theme-smoke: script done / 夹具就绪 在档）

用法：custom_theme_check.py <a.ppm> <b.ppm> <c.ppm> <log>
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from theme_visual_check import read_p6, detect_window  # noqa: E402

# ── 阈值（2026-11-23 标定）──
# 采样区 = 编辑器主体（user: editor 槽生效面）。色值经 parse_hsla
# 灰度近似装载（G4 已登记语义：h=0/s=0 只留 l）——蓝调不成立，
# 断言用**亮度判异**（A 帧中灰亮 vs C 帧近黑）。
SAMPLE_X = (300, 1200)   # 编辑器区（避开左右栏与顶栏）
SAMPLE_Y = (140, 700)
LUMA_MIN_A = 60.0        # A 帧编辑区均亮下限（editor 槽 #2447a0→l≈0.42≈107 生效）
LUMA_MAX_C = 60.0        # C 帧编辑区均亮上限（dark 基底近黑 ~35）
LUMA_DELTA = 25.0        # A-C 均亮差下限（四态→三态真实切换判异）


def rgb_at(px, w, x, y):
    o = (y * w + x) * 3
    return px[o], px[o + 1], px[o + 2]


def region_luma(px, w, x0, x1, y0, y1):
    """区域平均亮度（0-255；步进 4——menu_check 采样范式）。"""
    total = luma = 0
    for y in range(y0, y1, 4):
        for x in range(x0, x1, 4):
            r, g, b = rgb_at(px, w, x, y)
            total += 1
            luma += 0.299 * r + 0.587 * g + 0.114 * b
    return luma / total


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

    # C1 原点一致
    same = all(abs(o[0] - x0) <= 2 and abs(o[1] - y0) <= 2 for o in origins)
    check("C1-三帧窗口原点一致", same,
          " / ".join(f"({o[0]},{o[1]})" for o in origins))

    sx0, sx1 = x0 + SAMPLE_X[0], x0 + SAMPLE_X[1]
    sy0, sy1 = y0 + SAMPLE_Y[0], y0 + SAMPLE_Y[1]

    # C2 自定义色应用（A 帧编辑区亮度——editor 槽生效）
    la = region_luma(ap, w, sx0, sx1, sy0, sy1)
    check("C2-自定义色应用-A帧亮度", la >= LUMA_MIN_A,
          f"A 帧编辑区均亮 {la:.1f} ≥ {LUMA_MIN_A:.0f}（editor 槽生效——灰度装载语义）")

    # C3 dark 回退（C 帧近黑）
    lc = region_luma(cp, w, sx0, sx1, sy0, sy1)
    check("C3-dark回退-C帧近黑", lc <= LUMA_MAX_C,
          f"C 帧编辑区均亮 {lc:.1f} ≤ {LUMA_MAX_C:.0f}（dark 基底近黑）")

    # C4 四态→三态真实切换（A 亮度 - C 亮度 判异）
    check("C4-四态切三态-A-C亮度差", la - lc >= LUMA_DELTA,
          f"A {la:.1f} - C {lc:.1f} = {la-lc:.1f} ≥ {LUMA_DELTA:.0f}（user: → dark 判异）")

    # C5 证据行
    with open(log_path, encoding="utf-8", errors="replace") as f:
        log = f.read()
    has_done = "custom-theme-smoke: script done" in log
    has_fixture = "custom-theme-smoke: 夹具就绪" in log
    check("C5-证据行", has_done and has_fixture,
          f"done={'✓' if has_done else '✗'} / fixture={'✓' if has_fixture else '✗'}")

    for ln in lines:
        print(ln)
    tag = "OK" if not fails else f"FAIL: {fails}"
    print(f"custom-theme-check: {len(lines)} 项断言，{tag}（{a_path.rsplit('/',1)[-1]} 系 {w}x{h}）")
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
