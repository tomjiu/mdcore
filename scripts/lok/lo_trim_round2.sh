#!/usr/bin/env bash
# 第二轮：UI 配置面（soffice.cfg/modules 非 Writer 模块）——LOK 无头面
# 理论上不吃 UI 配置，但 postUnoCommand 走的是 UNO 分发（不吃 .cfg）；
# 实测为准：删非 Writer 模块 cfg + 资产 zip，门红即回滚。
set -uo pipefail

STAGE=/home/z/my-project/tmp/lo-trim
LO="$STAGE/lo-trim-live"
WORK="$STAGE/work"
HOLD="$STAGE/hold2"
GATE=/home/z/my-project/mdcore/scripts/lok/lo_trim_gate.sh
du_mb() { du -sm "$1" 2>/dev/null | awk '{print $1}'; }
mkdir -p "$HOLD"

try_delete() {
  local rel="$1" dest="$HOLD/$(echo "$1" | tr '/' '_')"
  [ -e "$LO/$rel" ] || return 0
  mkdir -p "$(dirname "$dest")"
  mv "$LO/$rel" "$dest"
  if bash "$GATE" "$LO" "$WORK" >"$WORK/gate-r2.log" 2>&1; then
    echo "R2 item=$rel status=KEEP_DELETED"
    rm -rf "$dest"
    return 0
  else
    echo "R2 item=$rel status=REQUIRED_ROLLBACK"
    mv "$dest" "$LO/$rel"
    return 1
  fi
}

start=$(du_mb "$LO")
echo "R2 start_mb=$start"

# 非 Writer 模块 UI 配置（calc/impress/draw/base/math 等已删代码库的对应 cfg）
for m in scalc simpress sdraw smath sabpilot sbibliography schart scanner \
         spropctrlr StartModule BasicIDE dbapp dbbrowser dbquery dbrelation \
         dbtable dbtdata sglobal sweb swxform; do
  try_delete "share/config/soffice.cfg/modules/$m"
done

# 图标资产包（LOK paintTile 自绘像素不经图标主题）
for z in images_colibre_svg.zip images_colibre_dark_svg.zip \
         images_colibre_dark.zip images_colibre.zip; do
  try_delete "share/config/$z"
done
try_delete "share/config/wizard"

end=$(du_mb "$LO")
echo "R2 end_mb=$end"
