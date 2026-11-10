#!/usr/bin/env bash
# #47 批次 4（E-1）主题两态视觉验收烟雾——Xvfb 双态截图 + PPM 像素断言。
#
# 用法:
#   ./theme_visual_smoke.sh [--out DIR] [--sec N] [--keep-display]
#
# 产物（--out DIR，默认 mang-shell/target/theme-visual/）:
#   theme-light.ppm / theme-dark.ppm   ffmpeg x11grab 原始帧（P6）
#   theme-light.png / theme-dark.png   文档取证 PNG
#   theme-light.log / theme-dark.log   应用运行日志（theme:/quit 证据行）
#   evidence.txt                      汇总证据行
#
# 判定: exit 0 = 双态截帧成功 + 两日志含 "theme:"/"smoke: quit reached"
#       + tests/theme_visual_check.py 像素断言全过。
#
# 环境（同 mang-shell/smoke.sh 的自适应口径）:
#   - 宿主机自带 GTK dev / Vulkan ICD → 直接跑
#   - 无 root 沙箱 → 自动挂 $MANG_SYSROOT（默认 ~/gtk-prefix，兼容
#     ~/gtk-prefix 与 ~/.local/sysroot 两个用户态前缀）的
#     pkg-config / 运行库 / lavapipe ICD 面
#   - 截帧依赖 ffmpeg x11grab（构建面已验证在 Debian 13 主线可用）
set -euo pipefail
cd "$(dirname "$0")"

OUT="mang-shell/target/theme-visual"
SEC=8
KEEP_DISPLAY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --sec) SEC="$2"; shift 2 ;;
    --keep-display) KEEP_DISPLAY=1; shift ;;
    *) echo "未知参数: $1" >&2; exit 64 ;;
  esac
done
OUT="$(mkdir -p "$OUT" && cd "$OUT" && pwd)"

# ── 用户态 sysroot 探测（MANG_SYSROOT > ~/gtk-prefix > ~/.local/sysroot）──
pick_prefix() {
  for cand in "${MANG_SYSROOT:-}" "$HOME/gtk-prefix" "$HOME/.local/sysroot"; do
    [ -n "$cand" ] && [ -d "$cand/usr/lib/x86_64-linux-gnu" ] && { echo "$cand"; return; }
  done
  echo ""
}
SYSROOT="$(pick_prefix)"
if [ -n "$SYSROOT" ]; then
  export PKG_CONFIG_PATH="$SYSROOT/usr/lib/x86_64-linux-gnu/pkgconfig:$SYSROOT/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
  export LD_LIBRARY_PATH="$SYSROOT/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  export LIBRARY_PATH="$SYSROOT/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"
  [ -d "$SYSROOT/usr/bin" ] && export PATH="$SYSROOT/usr/bin:$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"

# ── Vulkan ICD：宿主系统优先，其次 sysroot lavapipe ──
if [ -z "${VK_DRIVER_FILES:-}" ]; then
  if [ -f /usr/share/vulkan/icd.d/lvp_icd.json ]; then
    :
  elif [ -n "$SYSROOT" ] && [ -f "$SYSROOT/usr/share/vulkan/icd.d/lvp_icd.json" ]; then
    export VK_DRIVER_FILES="$SYSROOT/usr/share/vulkan/icd.d/lvp_icd.json"
  fi
fi

command -v ffmpeg >/dev/null || { echo "缺 ffmpeg（x11grab 截帧链）" >&2; exit 69; }
command -v Xvfb  >/dev/null || { echo "缺 Xvfb" >&2; exit 69; }

# 1280×800 窗口 + 服务端装饰边框在无 WM Xvfb 下起于 ~(+104,+56)，
# 屏幕需 ≥1600×1000 才能完整容窗（旧 #270 验收截图同口径）。
W=1600; H=1000
FIXTURE="mang-shell/tests/fixtures/smoke.md"
[ -f "$FIXTURE" ] || { echo "缺夹具 $FIXTURE" >&2; exit 69; }

echo "[1/5] 构建 mang-shell（debug）"
cargo build --manifest-path mang-shell/Cargo.toml
BIN="mang-shell/target/debug/mang-shell"

DISPLAY_NUM="${MANG_DISPLAY:-99}"
rm -f "/tmp/.X$DISPLAY_NUM-lock" "/tmp/.X11-unix/X$DISPLAY_NUM" 2>/dev/null || true
XVFB_PID=""
cleanup() {
  [ "$KEEP_DISPLAY" = 1 ] && return
  [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT

echo "[2/5] 启动 Xvfb :$DISPLAY_NUM（${W}x${H}x24）"
Xvfb ":$DISPLAY_NUM" -screen 0 "${W}x${H}x24" >/tmp/theme-smoke-xvfb.log 2>&1 &
XVFB_PID=$!
export DISPLAY=":$DISPLAY_NUM"
sleep 1.5

run_one() { # $1 = light|dark
  local mode="$1"
  local home="/tmp/theme-smoke-home-$mode"
  rm -rf "$home"; mkdir -p "$home"
  local log="$OUT/theme-$mode.log"
  echo "  [$mode] 启动（--smoke-theme $mode --smoke-sec $SEC）"
  HOME="$home" "$BIN" --smoke-theme "$mode" --smoke-sec "$SEC" "$FIXTURE" >"$log" 2>&1 &
  local app_pid=$!
  # 切换在 t=1.0s 生效，t=3.5s 后渲染稳定，截帧（首帧丢弃再截，取稳定帧）。
  sleep 3.5
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/theme-$mode-first.ppm" -y
  sleep 0.5
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/theme-$mode.ppm" -y
  ffmpeg -hide_banner -loglevel error -i "$OUT/theme-$mode.ppm" "$OUT/theme-$mode.png" -y
  rm -f "$OUT/theme-$mode-first.ppm"
  # 等应用自然退出（--smoke-sec），校验证据行。
  wait "$app_pid" || { echo "  [$mode] 应用退出码非 0" >&2; return 1; }
  grep -q "^theme:" "$log" || { echo "  [$mode] 缺 theme: 证据行" >&2; return 1; }
  grep -q "smoke: quit reached" "$log" || { echo "  [$mode] 缺 smoke: quit reached" >&2; return 1; }
  echo "  [$mode] $(grep '^theme:' "$log" | tail -1) + quit reached ✓"
}

echo "[3/5] 双态截帧"
run_one light
run_one dark

# 批次 5（E-2）：hover/选中态三帧采集（A=3.4s 基态（park 4.1s 墙钟前）
# / B=5.4s 悬停后（hover ≈4.7s 墙钟 + 0.7s 稳定）/ C=7.5s 点击选中后
# （up ≈6.3s 墙钟 + 1.2s）；应用侧相对时间线见 --smoke-hover-state 注释）。
# 工作区夹具：真实 git 仓（2 文件 + 1 改动）——无工作区时文件树/git
# 面板同为近空态，H4「视图切换」断言会失真（实测 MAD 1.1）。
make_hover_workspace() { # $1 = dir
  local ws="$1"
  rm -rf "$ws"; mkdir -p "$ws"
  git -C "$ws" init -q
  git -C "$ws" config user.email probe@local
  git -C "$ws" config user.name probe
  echo "# 简介文档" > "$ws/alpha.md"
  printf '第一行\n第二行\n第三行\n' > "$ws/beta.txt"
  git -C "$ws" add . >/dev/null
  git -C "$ws" commit -qm baseline
  echo "第四行（未提交改动）" >> "$ws/beta.txt"   # git 面板有 1 项改动
}
run_hover_state() { # $1 = light|dark
  local mode="$1"
  local home="/tmp/theme-smoke-home-hover-$mode"
  local ws="/tmp/theme-smoke-ws-hover-$mode"
  rm -rf "$home"; mkdir -p "$home"
  command -v git >/dev/null || { echo "缺 git（工作区夹具）" >&2; return 1; }
  make_hover_workspace "$ws"
  local log="$OUT/hover-$mode.log"
  echo "  [hover-$mode] 启动（--smoke-hover-state $mode --smoke-workspace ws --smoke-sec 11）"
  HOME="$home" "$BIN" --smoke-hover-state "$mode" --smoke-sec 11 \
    --smoke-workspace "$ws" "$FIXTURE" >"$log" 2>&1 &
  local app_pid=$!
  # A=3.6s 基态（工作区 2.5s 开 + 渲染窗，park 5.1s 墙钟前）
  # / B=7.2s 悬停后（hover 5.7s 墙钟 + 1.5s——实测 +1.4s 已落像素）
  # / C=9.1s 点击选中后（up 8.05s 墙钟 + 1.0s）。
  sleep 3.6
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/hover-$mode-a.ppm" -y
  sleep 3.6
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/hover-$mode-b.ppm" -y
  sleep 1.9
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/hover-$mode-c.ppm" -y
  ffmpeg -hide_banner -loglevel error -i "$OUT/hover-$mode-c.ppm" "$OUT/hover-$mode.png" -y
  wait "$app_pid" || { echo "  [hover-$mode] 应用退出码非 0" >&2; return 1; }
  grep -q "hover-state-smoke: script done" "$log" \
    || { echo "  [hover-$mode] 缺 hover-state-smoke 证据行" >&2; return 1; }
  grep -q "smoke: quit reached" "$log" \
    || { echo "  [hover-$mode] 缺 smoke: quit reached" >&2; return 1; }
  echo "  [hover-$mode] 三帧采集 ✓ $(grep 'hover-state-smoke' "$log" | tail -1 | sed 's/^hover-state-smoke: //')"
}

echo "[4/5] hover/选中态三帧采集（双主题）"
run_hover_state light
run_hover_state dark

echo "[5/5] PPM 像素断言（E-1 双态 + E-2 hover/选中态）"
python3 mang-shell/tests/theme_visual_check.py \
  "$OUT/theme-light.ppm" "$OUT/theme-dark.ppm" | tee "$OUT/evidence.txt"
for mode in light dark; do
  python3 mang-shell/tests/hover_state_check.py \
    "$OUT/hover-$mode-a.ppm" "$OUT/hover-$mode-b.ppm" "$OUT/hover-$mode-c.ppm" \
    | tee -a "$OUT/evidence.txt"
done

echo "OK: 主题两态 + hover/选中态视觉证据 → $OUT"
