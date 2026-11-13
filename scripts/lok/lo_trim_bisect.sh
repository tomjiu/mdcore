#!/usr/bin/env bash
# B3/B4 细化轮：逐库（逐资产）二分定位可删子集。
# 粗批教训（2026-11-12 首轮）：34 库整批删 → smoke 红——粗批里混着必经库。
# 本轮：每次删一个单位 → 跑门 → 红即回滚该单位（保留其余已删面），
# 输出 per-item 裁决表 items.jsonl。
set -uo pipefail

STAGE=/home/z/my-project/tmp/lo-trim
LO="$STAGE/lo-trim-live"
WORK="$STAGE/work"
HOLD="$STAGE/hold"
ITEMS="$STAGE/items.jsonl"
GATE=/home/z/my-project/mdcore/scripts/lok/lo_trim_gate.sh

du_mb() { du -sm "$1" 2>/dev/null | awk '{print $1}'; }
mkdir -p "$HOLD"

# 候选单位清单（B3 库 + B4 资产；按预期收益降序——大头先试）
CANDIDATES=(
  # B4 资产面（首轮整批红，逐个定位）
  "program/classes"
  "program/__pycache__"
  "program/intro.png"
  "program/intro-highres.png"
  "program/gdbtrace"
  "program/java-set-classpath"
  # B3 非 Writer 模块库（首轮整批红，逐个定位）
  "program/libcalclo.so"
  "program/libsclo.so"
  "program/libscuilo.so"
  "program/libsvxlo.so"
  "program/libsd*lo.so"
  "program/libcuilo.so"
  "program/libbasctllo.so"
  "program/libchart*lo.so"
  "program/libavmediagst.so"
  "program/libOGLTranslo.so"
  "program/libPresentationMinimizerlo.so"
  "program/libabplo.so"
  "program/libacclo.so"
  "program/libanalysislo.so"
  "program/libanimcorelo.so"
  "program/libbiblo.so"
  "program/libcached1.so"
  "program/libdbalo.so"
  "program/libdbaselo.so"
  "program/libdbpool2.so"
  "program/libdbtoolslo.so"
  "program/libsdbtlo.so"
  "program/libdlgprovlo.so"
  "program/libflashlo.so"
  "program/libfrmlo.so"
  "program/libguesslanglo.so"
  "program/libhyaqsqlo.so"
  "program/libldapbe2lo.so"
  "program/libloglo.so"
  "program/liblnglo.so"
  "program/liboffacclo.so"
  "program/libpcrlo.so"
  "program/libpreloadlo.so"
  "program/libreslo.so"
  "program/librpdmgrlo.so"
  "program/librpt*lo.so"
  "program/libsaxlo.so"
  "program/libscript*lo.so"
  "program/libspelllo.so"
  "program/libswd*lo.so"
  "program/libtextconversiondictslx.so"
  "program/libunopkgapp.so"
  "program/libvbahelperlo.so"
  "program/libwriterfilterlo.so"
  "program/libwpft*lo.so"
  "program/libvclcanvaslo.so"
  "program/libcairocanvaslo.so"
  "program/libemb*lo.so"
  "program/libevtattlo.so"
)

start_mb=$(du_mb "$LO")
echo "start_mb=$start_mb" | tee "$ITEMS"

for pat in "${CANDIDATES[@]}"; do
  # shellcheck disable=SC2206
  matches=($LO/$pat)
  exists=0
  for m in "${matches[@]}"; do [ -e "$m" ] && exists=1 && break; done
  [ "$exists" = 1 ] || continue
  name=$(basename "$pat")
  # 删（挪到 hold）
  for m in "${matches[@]}"; do
    [ -e "$m" ] && mv "$m" "$HOLD/" 2>/dev/null
  done
  if bash "$GATE" "$LO" "$WORK" >"$WORK/gate-item.log" 2>&1; then
    echo "item=$pat status=KEEP_DELETED" | tee -a "$ITEMS"
    rm -rf "$HOLD"/*
  else
    echo "item=$pat status=REQUIRED_ROLLBACK" | tee -a "$ITEMS"
    # 回滚：挪回 program/
    for m in "$HOLD"/*; do
      [ -e "$m" ] && mv "$m" "$LO/program/"
    done
  fi
done

end_mb=$(du_mb "$LO")
echo "end_mb=$end_mb" | tee -a "$ITEMS"
echo "=== 细化轮终态：$start_mb → $end_mb MB ==="
grep -c "KEEP_DELETED" "$ITEMS" || true
grep -c "REQUIRED_ROLLBACK" "$ITEMS" || true
