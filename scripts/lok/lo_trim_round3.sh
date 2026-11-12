#!/usr/bin/env bash
# 第三轮：残留明细面（libscfilt=calc 过滤器 / libdbu=数据库 UI /
# localedata 非中英面 / vba 对象库 / sm-sd-smath 残库 / unopkg 向导面）。
# libmsword/libsw/libskia/libpdfium/libmergedlo = Writer 渲染内核必经，不动。
set -uo pipefail

STAGE=/home/z/my-project/tmp/lo-trim
LO="$STAGE/lo-trim-live"
WORK="$STAGE/work"
HOLD="$STAGE/hold3"
GATE=/home/z/my-project/mdcore/scripts/lok/lo_trim_gate.sh
du_mb() { du -sm "$1" 2>/dev/null | awk '{print $1}'; }
mkdir -p "$HOLD"

try_delete() {
  local rel="$1" dest="$HOLD/$(echo "$1" | tr '/' '_')"
  [ -e "$LO/$rel" ] || return 0
  mkdir -p "$(dirname "$dest")"
  mv "$LO/$rel" "$dest"
  if bash "$GATE" "$LO" "$WORK" >"$WORK/gate-r3.log" 2>&1; then
    echo "R3 item=$rel status=KEEP_DELETED"
    rm -rf "$dest"
    return 0
  else
    echo "R3 item=$rel status=REQUIRED_ROLLBACK"
    mv "$dest" "$LO/$rel"
    return 1
  fi
}

start=$(du_mb "$LO")
echo "R3 start_mb=$start"

# calc 过滤器库（mdcore LOK 面无 xlsx→LO 交互；saveAs 只走 docx/pdf）
try_delete "program/libscfiltlo.so"
# 数据库 UI（base 面残留）
try_delete "program/libdbulo.so"
# locale 非核心面（zh/en 欧面留，others=阿拉伯/泰等 41 locale）
try_delete "program/liblocaledata_others.so"
# VBA 兼容对象库（Writer 的 vbaswobj 可能经 msword 面——实测为准）
try_delete "program/libvbaobjlo.so"
try_delete "program/libvbaswobjlo.so"
# smath 库（公式编辑器内核）
try_delete "program/libsmlo.so"
# unopkg 面（扩展管理器 CLI——LOK 零扩展）
try_delete "program/unopkg"
try_delete "program/unopkg.bin"
try_delete "program/unoinfo"
# xpdfimport（PDF 导入转换——mdcore PDF 面走 pdfium，不走 LO 导入）
try_delete "program/xpdfimport"
# wizards 产物（向导脚本面）
try_delete "program/wizards"

end=$(du_mb "$LO")
echo "R3 end_mb=$end"
