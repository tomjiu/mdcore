#!/usr/bin/env bash
# rust-env.sh —— mdcore 沙箱环境自愈手册（单一真源：本文件）
#
# 背景：云沙箱默认无 Rust 工具链，且会被周期性回收（2026-11-11 单日双回收
# 实测：清掉 ~/.cargo ~/.rustup 与 /tmp 工具缓存，恢复序列每次人工重演
# 30+ 分钟——#151）。本脚本把「从裸沙箱到四条门可跑」固化成一条命令。
# 姐妹仓先例：ai-bid-research `91fb67b` scripts/rust-env.sh（同模式移植，
# mdcore 四件套适配：GPUI 链接系库 / pdfium / mingw 交叉 / NSIS 打包）。
#
# 用法（bash scripts/rust-env.sh，幂等——每步先探测已存在）：
#   ① Rust 工具链：rustup + 仓 pin（rust-toolchain.toml channel 解析安装，
#      不硬编码版本——pin 升级零维护）+ wasm32 target + fmt/clippy 组件
#   ② GPUI Linux 链接系库免 root 自愈：apt-get download libxkbcommon{0,-dev,
#      -x11-0,-x11-dev} → dpkg -x 解到 $TOOLSROOT/syslibs（无 sudo 的链接面；
#      LIBRARY_PATH/LD_LIBRARY_PATH 导出口径——ACCEPTANCE-270 §构建 同源）
#   ③ pdfium 取件（docs/INSTALL.md 口径）：chromium/8125 release tgz →
#      $TOOLSROOT/pdfium；MDC_PDFIUM_LIB 指路（引擎门控测试真跑用）
#   ④ --with-mingw：Windows 交叉检查链（binutils windres + gcc-14-posix +
#      CRT dev 树 + -posix 后缀名→无后缀符号链接 + rustup windows-gnu target）
#   ⑤ --with-nsis：NSIS 打包验证链（nsis + nsis-common 解包，NSISDIR/PATH
#      指路——BUILD-VERIFY §3「Debian nsis 3.11 + NSISDIR 指路」同口径）
#
# 环境变量（source 执行时直接导出；bash 执行时末尾打印导出块）：
#   TOOLSROOT        工具树根（默认 ~/.mang-tools）
#   LIBRARY_PATH / LD_LIBRARY_PATH   ②的链接/运行面
#   PKG_CONFIG_PATH  ②的 .pc 面（zed 闭包个别 build script 用）
#   MDC_PDFIUM_LIB   ③的引擎指路
#   PATH             ④/⑤ 的工具面
#   CARGO_PROFILE_DEV_DEBUG=0 / CARGO_PROFILE_TEST_DEBUG=0
#     磁盘纪律（ai-bid 同口径）：debuginfo 关闭后链接产物 ~40MB vs ~300MB；
#     9.9G 盘装得下全树测试。开着 debuginfo 链接第 ~30 个测试二进制即
#     Bus error/ENOSPC（实测教训）。
#
# 四条门的对照（2026-11-11 裸沙箱实测，2 核沙箱）：
#   cargo test -p mdshell --test r24_packaging   ①即可，~30s
#   cargo test -p mang-shell --lib --bins        ①+②，首建 ~10min（+③引擎门
#     测试真跑；无 ③ 该面 SKIP 不算失败）
#   cargo check -p mang-shell --target x86_64-pc-windows-gnu   ①+④，~10min
#   packaging/nsis/build-installer.py + 礼貌注册测试              ①+⑤，~1min
set -euo pipefail

TOOLSROOT="${TOOLSROOT:-$HOME/.mang-tools}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WITH_MINGW=0; WITH_NSIS=0
for a in "$@"; do
  case "$a" in
    --with-mingw) WITH_MINGW=1 ;;
    --with-nsis)  WITH_NSIS=1 ;;
    *) echo "rust-env: 未知参数 $a（可用：--with-mingw --with-nsis）" >&2; exit 2 ;;
  esac
done

log() { printf '[rust-env] %s\n' "$*"; }

# source 检测（被 source 时直接导出；被执行时末尾打印导出块）
SOURCED=0
[ "${BASH_SOURCE[0]}" != "$0" ] && SOURCED=1

export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0

# ---- ① Rust 工具链（pin 解析自 rust-toolchain.toml，不硬编码）------------
if ! command -v rustup >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  log "① 安装 rustup（minimal）"
  curl -sSfL https://sh.rustup.rs -o /tmp/rustup-init.sh
  sh /tmp/rustup-init.sh -y --profile minimal --no-modify-path
  export PATH="$HOME/.cargo/bin:$PATH"
else
  log "① rustup 已在位"
  command -v cargo >/dev/null 2>&1 || export PATH="$HOME/.cargo/bin:$PATH"
fi

cd "$REPO"
CHANNEL="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml | head -1)"
if [ -z "$CHANNEL" ]; then
  log "① rust-toolchain.toml 无 channel——跳过 pin 安装（用默认工具链）"
else
  if rustup toolchain list | grep -q "^$CHANNEL"; then
    log "① 工具链 $CHANNEL 已在位（rust-toolchain.toml pin）"
  else
    log "① 安装 pin 工具链 $CHANNEL（+ rustfmt/clippy 组件）"
    rustup toolchain install "$CHANNEL" --profile minimal --component rustfmt --component clippy
  fi
  rustup target list --installed --toolchain "$CHANNEL" | grep -q wasm32-unknown-unknown \
    || rustup target add wasm32-unknown-unknown --toolchain "$CHANNEL" \
    || log "① wasm32 target 安装失败（wasm32 check 门需要时再装）"
fi
log "① rustc $(rustc --version 2>/dev/null || echo 未就绪——检查 rust-toolchain.toml)"

# ---- ② GPUI Linux 链接系库（免 root：download + dpkg -x）------------------
SYSLIBS="$TOOLSROOT/syslibs"
want_lib() {  # want_lib <soname> —— ldconfig 探测系统已有则免装
  ldconfig -p 2>/dev/null | grep -q "$1" && return 0
  [ -e "$SYSLIBS/root/usr/lib/x86_64-linux-gnu/$1" ] && return 0
  return 1
}
if want_lib libxkbcommon.so.0 && want_lib libxkbcommon-x11.so.0; then
  log "② xkbcommon 链接系库已在位（系统或 $SYSLIBS）"
else
  log "② 下载 libxkbcommon{0,-dev,-x11-0,-x11-dev} → $SYSLIBS"
  mkdir -p "$SYSLIBS/debs"; cd "$SYSLIBS/debs"
  apt-get download libxkbcommon0 libxkbcommon-dev libxkbcommon-x11-0 libxkbcommon-x11-dev
  for d in ./*.deb; do dpkg -x "$d" "$SYSLIBS/root"; done
  rm -rf "$SYSLIBS/debs"
  cd "$REPO"
fi
export LIBRARY_PATH="$SYSLIBS/root/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$SYSLIBS/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG_PATH="$SYSLIBS/root/usr/lib/x86_64-linux-gnu/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

# ---- ③ pdfium（chromium/8125 release，docs/INSTALL.md 口径）---------------
PDFIUM_DIR="$TOOLSROOT/pdfium/chromium-8125"
if [ -f "$PDFIUM_DIR/lib/libpdfium.so" ]; then
  log "③ pdfium 已在位（$PDFIUM_DIR/lib/libpdfium.so）"
else
  log "③ 下载 pdfium chromium/8125 linux-x64（~3.8MB tgz）"
  mkdir -p "$TOOLSROOT/dl"; cd "$TOOLSROOT/dl"
  curl -sSfL -o pdfium-linux-x64.tgz \
    "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8125/pdfium-linux-x64.tgz"
  mkdir -p "$PDFIUM_DIR"
  tar xzf pdfium-linux-x64.tgz -C "$PDFIUM_DIR"
  rm -f pdfium-linux-x64.tgz
  cd "$REPO"
fi
export MDC_PDFIUM_LIB="$PDFIUM_DIR/lib/libpdfium.so"

# ---- ④ Windows 交叉检查链（--with-mingw）----------------------------------
if [ "$WITH_MINGW" = 1 ]; then
  MINGW="$TOOLSROOT/mingw"
  if [ -x "$MINGW/usr/bin/x86_64-w64-mingw32-windres" ]; then
    log "④ mingw 交叉树已在位"
  else
    log "④ 下载 mingw-w64 工具链（binutils windres + gcc-14-posix + CRT dev）"
    mkdir -p "$MINGW/debs"; cd "$MINGW/debs"
    apt-get download binutils-mingw-w64-x86-64 \
      gcc-mingw-w64-x86-64-posix gcc-mingw-w64-x86-64-posix-runtime \
      gcc-mingw-w64-base mingw-w64-x86-64-dev
    for d in ./*.deb; do dpkg -x "$d" "$MINGW"; done
    rm -rf "$MINGW/debs"
    cd "$REPO"
  fi
  # windres 的预处理期找无后缀名 gcc——-posix 变体实际提供，补符号链接
  for t in gcc cpp gcc-ar gcc-ranlib; do
    if [ -x "$MINGW/usr/bin/x86_64-w64-mingw32-$t-posix" ] \
       && [ ! -e "$MINGW/usr/bin/x86_64-w64-mingw32-$t" ]; then
      ln -sf "$MINGW/usr/bin/x86_64-w64-mingw32-$t-posix" \
             "$MINGW/usr/bin/x86_64-w64-mingw32-$t"
    fi
  done
  export PATH="$MINGW/usr/bin:$PATH"
  if [ -n "${CHANNEL:-}" ]; then
    rustup target list --installed --toolchain "$CHANNEL" | grep -q x86_64-pc-windows-gnu \
      || rustup target add x86_64-pc-windows-gnu --toolchain "$CHANNEL"
  else
    rustup target add x86_64-pc-windows-gnu
  fi
  log "④ windres $(x86_64-w64-mingw32-windres --version | head -1) / gcc $(x86_64-w64-mingw32-gcc --version | head -1)"
fi

# ---- ⑤ NSIS 打包验证链（--with-nsis）--------------------------------------
if [ "$WITH_NSIS" = 1 ]; then
  NSIS="$TOOLSROOT/nsis"
  if [ -x "$NSIS/usr/bin/makensis" ] && [ -d "$NSIS/usr/share/nsis" ]; then
    log "⑤ NSIS 树已在位"
  else
    log "⑤ 下载 nsis + nsis-common（Debian 3.11）"
    mkdir -p "$NSIS/debs"; cd "$NSIS/debs"
    apt-get download nsis nsis-common
    for d in ./*.deb; do dpkg -x "$d" "$NSIS"; done
    rm -rf "$NSIS/debs"
    cd "$REPO"
  fi
  export NSISDIR="$NSIS/usr/share/nsis"
  export PATH="$NSIS/usr/bin:$PATH"
  log "⑤ makensis $(makensis -VERSION)"
fi

# ---- 导出面 ----------------------------------------------------------------
ENVBLOCK=$(cat <<ENVECHO
export TOOLSROOT="$TOOLSROOT"
export LIBRARY_PATH="$SYSLIBS/root/usr/lib/x86_64-linux-gnu\${LIBRARY_PATH:+:\$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$SYSLIBS/root/usr/lib/x86_64-linux-gnu\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
export PKG_CONFIG_PATH="$SYSLIBS/root/usr/lib/x86_64-linux-gnu/pkgconfig\${PKG_CONFIG_PATH:+:\$PKG_CONFIG_PATH}"
export MDC_PDFIUM_LIB="$PDFIUM_DIR/lib/libpdfium.so"
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
ENVECHO
)
if [ "$SOURCED" = 1 ]; then
  eval "$ENVBLOCK"
  log "完成（已 source 导出）。四条门：r24 门 / mang-shell 测试 / windows-gnu check / NSIS 验证"
else
  log "完成。后续 shell 导出（或本脚本 source 执行即自动导出）："
  printf '%s\n' "$ENVBLOCK"
  EXTRA_PATH=""
  [ "$WITH_MINGW" = 1 ] && EXTRA_PATH="$TOOLSROOT/mingw/usr/bin"
  [ "$WITH_NSIS" = 1 ] && EXTRA_PATH="${EXTRA_PATH:+$EXTRA_PATH:}$TOOLSROOT/nsis/usr/bin"
  [ -n "$EXTRA_PATH" ] && printf 'export PATH="%s:$PATH"  # mingw/nsis 工具面（--with-* 时）\n' "$EXTRA_PATH"
  log "四条门对照见文件头注释；pdfium 引擎测试 = MDC_PDFIUM_LIB 已指路"
fi
