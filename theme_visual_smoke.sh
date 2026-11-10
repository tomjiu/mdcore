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

echo "[1/8] 构建 mang-shell（debug）"
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

echo "[2/8] 启动 Xvfb :$DISPLAY_NUM（${W}x${H}x24）"
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

echo "[3/8] 双态截帧"
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

echo "[4/8] hover/选中态三帧采集（双主题）"
run_hover_state light
run_hover_state dark

# 批次 6（E-2 禁用态）：--smoke-disabled-state 双面跑——禁用面（非 git
# 工作区：面板构造但不可用→刷新钮禁用）/ 对照面（git 工作区）。
# 外层 A@5.8s / B@8.2s / C@10.0s。
run_disabled_state() { # $1 = disabled|control  $2 = light|dark
  local face="$1" mode="$2"
  local home="/tmp/theme-smoke-home-ds-$face"
  local log="$OUT/ds-$face.log"
  rm -rf "$home"; mkdir -p "$home"
  local ws_args=()
  command -v git >/dev/null || { echo "缺 git（工作区夹具）" >&2; return 1; }
  if [ "$face" = "control" ]; then
    local ws="/tmp/theme-smoke-ws-ds-$mode"
    make_hover_workspace "$ws"
    ws_args=(--smoke-workspace "$ws")
  else
    # 禁用面：非 git 工作区目录——GitPanel 随工作区构造（shell.rs
    # open_workspace「工作区打开即建」），root 非 git 仓 → 不可用态 →
    # 刷新钮禁用。无工作区时面板不构造，测不到禁用面。
    local ws="/tmp/theme-smoke-ws-plain-$mode"
    rm -rf "$ws"; mkdir -p "$ws/子目录"
    echo "# 非 git 工作区" > "$ws/普通文档.md"
    ws_args=(--smoke-workspace "$ws")
  fi
  echo "  [ds-$face] 启动（--smoke-disabled-state $mode --smoke-sec 12 ${ws_args[*]:+工作区}）"
  HOME="$home" "$BIN" --smoke-disabled-state "$mode" --smoke-sec 12 \
    "${ws_args[@]}" >"$log" 2>&1 &
  local app_pid=$!
  # A=5.8s（act-git 点击 3.5s 墙钟后 + 渲染窗，park 5.1s 后）
  # / B=8.2s（hover 6.7s 墙钟 + 1.5s）/ C=10.0s（up 8.65s 墙钟 + 1.35s）。
  sleep 5.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/ds-$face-a.ppm" -y
  sleep 2.4
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/ds-$face-b.ppm" -y
  sleep 1.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/ds-$face-c.ppm" -y
  ffmpeg -hide_banner -loglevel error -i "$OUT/ds-$face-c.ppm" "$OUT/ds-$face.png" -y
  wait "$app_pid" || { echo "  [ds-$face] 应用退出码非 0" >&2; return 1; }
  grep -q "disabled-state-smoke: script done" "$log" \
    || { echo "  [ds-$face] 缺 disabled-state-smoke 证据行" >&2; return 1; }
  grep -q "smoke: quit reached" "$log" \
    || { echo "  [ds-$face] 缺 smoke: quit reached" >&2; return 1; }
  echo "  [ds-$face] 三帧采集 ✓"
}

echo "[5/8] 禁用态双面采集（disabled / control）"
run_disabled_state disabled light
run_disabled_state control light

# 批次 7（A4 菜单）：--smoke-menu 三帧（A=5.8s 基态 / B=7.6s 开菜单
# （点击 6.7s 墙钟 + 0.9s）/ C=9.4s（Esc 8.65s 墙钟 + 0.75s）——菜单
# 开关由点击/Esc 同步驱动，外层窗为帧呈现裕量）。
run_menu() { # $1 = light|dark
  local mode="$1"
  local home="/tmp/theme-smoke-home-menu-$mode"
  local log="$OUT/menu-$mode.log"
  rm -rf "$home"; mkdir -p "$home"
  echo "  [menu-$mode] 启动（--smoke-menu $mode --smoke-sec 11）"
  HOME="$home" "$BIN" --smoke-menu "$mode" --smoke-sec 11 "$FIXTURE" >"$log" 2>&1 &
  local app_pid=$!
  sleep 5.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/menu-$mode-a.ppm" -y
  sleep 1.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/menu-$mode-b.ppm" -y
  sleep 1.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/menu-$mode-c.ppm" -y
  ffmpeg -hide_banner -loglevel error -i "$OUT/menu-$mode-c.ppm" "$OUT/menu-$mode.png" -y
  wait "$app_pid" || { echo "  [menu-$mode] 应用退出码非 0" >&2; return 1; }
  grep -q "menu-smoke: script done" "$log" \
    || { echo "  [menu-$mode] 缺 menu-smoke 证据行" >&2; return 1; }
  grep -q "smoke: quit reached" "$log" \
    || { echo "  [menu-$mode] 缺 smoke: quit reached" >&2; return 1; }
  echo "  [menu-$mode] 三帧采集 ✓"
}

echo "[6/8] 菜单开/关三帧采集（A4）"
run_menu light

# 批次 8（A4 右键菜单）：--smoke-context-menu 三帧（A=5.8s / B=7.6s
#（右键 6.7s 墙钟 + 0.9s）/ C=9.4s（点外 8.65s + 0.75s））。
run_context_menu() { # $1 = light|dark
  local mode="$1"
  local home="/tmp/theme-smoke-home-cm-$mode"
  local log="$OUT/cm-$mode.log"
  rm -rf "$home"; mkdir -p "$home"
  echo "  [cm-$mode] 启动（--smoke-context-menu $mode --smoke-sec 11）"
  HOME="$home" "$BIN" --smoke-context-menu "$mode" --smoke-sec 11 "$FIXTURE" >"$log" 2>&1 &
  local app_pid=$!
  sleep 5.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/cm-$mode-a.ppm" -y
  sleep 1.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/cm-$mode-b.ppm" -y
  sleep 1.8
  ffmpeg -hide_banner -loglevel error -f x11grab -video_size "${W}x${H}" \
    -i "$DISPLAY" -frames:v 1 -pix_fmt rgb24 "$OUT/cm-$mode-c.ppm" -y
  ffmpeg -hide_banner -loglevel error -i "$OUT/cm-$mode-c.ppm" "$OUT/cm-$mode.png" -y
  wait "$app_pid" || { echo "  [cm-$mode] 应用退出码非 0" >&2; return 1; }
  grep -q "context-menu-smoke: script done" "$log" \
    || { echo "  [cm-$mode] 缺 context-menu-smoke 证据行" >&2; return 1; }
  grep -q "smoke: quit reached" "$log" \
    || { echo "  [cm-$mode] 缺 smoke: quit reached" >&2; return 1; }
  echo "  [cm-$mode] 三帧采集 ✓"
}

echo "[7/8] 右键菜单开/关三帧采集（A4）"
run_context_menu light

echo "[8/8] PPM 像素断言（E-1 + E-2 + A4 菜单/右键）"
# 断言链容错：单项失败记录到 FAILURES 不中止——全部跑完统一判定
#（set -o pipefail 下 tee 管道会传递检查脚本退出码）。
run_check() { # $@ = 检查脚本 + 参数
  if ! "$@" | tee -a "$OUT/evidence.txt"; then
    echo "CHECK-FAILED: $*" >> "$OUT/evidence.txt"
    CHECK_FAILED=1
  fi
}
: > "$OUT/evidence.txt"
run_check python3 mang-shell/tests/theme_visual_check.py \
  "$OUT/theme-light.ppm" "$OUT/theme-dark.ppm"
for mode in light dark; do
  run_check python3 mang-shell/tests/hover_state_check.py \
    "$OUT/hover-$mode-a.ppm" "$OUT/hover-$mode-b.ppm" "$OUT/hover-$mode-c.ppm"
done
for face in disabled control; do
  run_check python3 mang-shell/tests/disabled_state_check.py "$face" \
    "$OUT/ds-$face-a.ppm" "$OUT/ds-$face-b.ppm" "$OUT/ds-$face-c.ppm" "$OUT/ds-$face.log"
done
for mode in light; do
  run_check python3 mang-shell/tests/menu_check.py \
    "$OUT/menu-$mode-a.ppm" "$OUT/menu-$mode-b.ppm" "$OUT/menu-$mode-c.ppm" "$OUT/menu-$mode.log"
done
for mode in light; do
  run_check python3 mang-shell/tests/context_menu_check.py \
    "$OUT/cm-$mode-a.ppm" "$OUT/cm-$mode-b.ppm" "$OUT/cm-$mode-c.ppm" "$OUT/cm-$mode.log"
done

if [ "${CHECK_FAILED:-0}" = "1" ]; then
  echo "存在失败断言（详见 $OUT/evidence.txt 的 CHECK-FAILED 行）" >&2
  exit 1
fi
echo "OK: 主题两态 + hover/选中态 + 禁用态 + 菜单/右键菜单开/关视觉证据 → $OUT"
