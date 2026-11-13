#!/usr/bin/env bash
# LO 裁剪实验主驱动（2026-11-12 重做轮）。
#
# 流程：基线门（已过）→ 按依赖序分批删减 → 每批后跑 lo_trim_gate.sh →
#   全绿记录体积 / 红则回滚该批（mv 回来）并标记「不可删」。
# 产物：batches.jsonl（每批：名称/删除面/体积前后/门结果/回滚与否）。
#
# 批次主题（从资源类到模块库，先外围后内核）：
#   B1 share 资源类：gallery(14M) template(9M) wizards(5M) basic(5M)
#      xslt(5M) palette(2M) wordbook labels tipoftheday uno_packages
#   B2 share/config 大头里的非 Writer 面：gallery 后的 so_kimages 等配置
#      （config 37M 里主删非 Writer 模块位）
#   B3 program 非 Writer 模块库：calc/impress/draw/base/math 面 so
#      （writer 核心 libsw* writerfilter odf 面保留）
#   B4 program 明细回收：java classes/ 明细 png intro 等资产面
set -euo pipefail

STAGE=/home/z/my-project/tmp/lo-trim
LO="$STAGE/lo-trim-live"      # 工作副本（从 baseline 复制一次，逐批删）
WORK="$STAGE/work"
RECYCLE="$STAGE/recycled"     # 每批删除物暂存（红门回滚用 + 最终统计）
JSONL="$STAGE/batches.jsonl"

GATE=/home/z/my-project/mdcore/scripts/lok/lo_trim_gate.sh
LO_ROOT="$LO"

du_mb() { du -sm "$1" 2>/dev/null | awk '{print $1}'; }
log()  { echo "$*" | tee -a "$JSONL"; }

[ -d "$LO" ] || cp -a "$STAGE/lo-baseline" "$LO"
mkdir -p "$WORK" "$RECYCLE"

# 每批定义：名称 + 删什么（相对 $LO 的 glob 列表）
run_batch() {
  local name="$1"; shift
  local globs=("$@")
  local before=$(du_mb "$LO")
  local batch_dir="$RECYCLE/$name"
  mkdir -p "$batch_dir"
  local deleted=0
  for g in "${globs[@]}"; do
    # shellcheck disable=SC2206
    local matches=($LO/$g)
    for m in "${matches[@]}"; do
      if [ -e "$m" ]; then
        mv "$m" "$batch_dir/" && deleted=$((deleted+1))
      fi
    done
  done
  local after=$(du_mb "$LO")
  if bash "$GATE" "$LO" "$WORK" >"$WORK/gate-$name.log" 2>&1; then
    log "batch=$name status=PASS deleted_entries=$deleted mb_before=$before mb_after=$after"
    echo "[trim] $name: PASS ($before → $after MB, $deleted 项)"
    # PASS 的批：真正删除（回收站清该批）
    rm -rf "$batch_dir"
    return 0
  else
    log "batch=$name status=FAIL_ROLLED_BACK deleted_entries=$deleted mb_before=$before mb_after=$after"
    echo "[trim] $name: FAIL → 回滚"
    # 回滚：把该批挪回原位
    for m in "$batch_dir"/*; do
      [ -e "$m" ] || continue
      local base=$(basename "$m")
      # 顶层条目直接回；嵌套目录位（share/xxx）需复原路径
      case "$base" in
        *) mv "$m" "$LO/${parent_map[$name]:-}/$base" 2>/dev/null || mv "$m" "$LO/$base" ;;
      esac
    done
    rm -rf "$batch_dir"
    return 1
  fi
}

# 嵌套父路径表（回滚复原用）
declare -A parent_map
parent_map[b1_share_resources]="share"
parent_map[b2_config_nonwriter]="share/config"
parent_map[b3_program_nonwriter]="program"
parent_map[b4_program_assets]="program"

# ---- B1：share 资源类（与代码无关的美工/模板/宏/向导资产）----
run_batch b1_share_resources \
  "share/gallery" "share/template" "share/wizards" "share/basic" \
  "share/xslt" "share/palette" "share/wordbook" "share/labels" \
  "share/tipoftheday" "share/uno_packages" "share/glade" \
  "share/toolbarmode" "share/theme_definitions" || true

# ---- B2：share/config 里非 Writer 面（先探 config 结构再定）----
ls "$LO/share/config" > "$WORK/config-listing.txt" 2>/dev/null || true

# ---- B3：program 非 Writer 模块库（计算/演示/绘图/数据库/公式）----
run_batch b3_program_nonwriter \
  "program/libcalclo.so" "program/libsclo.so" "program/libscuilo.so" \
  "program/libchart*lo.so" "program/libavmediagst.so" \
  "program/libOGLTranslo.so" "program/libPresentationMinimizerlo.so" \
  "program/libabplo.so" "program/libacclo.so" "program/libanalysislo.so" \
  "program/libanimcorelo.so" "program/libbasctllo.so" "program/libbiblo.so" \
  "program/libcached1.so" "program/libcuilo.so" "program/libdbalo.so" \
  "program/libdbaselo.so" "program/libdbpool2.so" "program/libdbtoolslo.so" \
  "program/libsdbtlo.so" "program/libdlgprovlo.so" "program/libflashlo.so" \
  "program/libfrmlo.so" "program/libguesslanglo.so" "program/libhyaqsqlo.so" \
  "program/libldapbe2lo.so" "program/libloglo.so" "program/liblnglo.so" \
  "program/liboffacclo.so" "program/libpcrlo.so" "program/libpreloadlo.so" \
  "program/libreslo.so" "program/librpdmgrlo.so" "program/librpt*lo.so" \
  "program/libsaxlo.so" "program/libscript*lo.so" "program/libsd*lo.so" \
  "program/libspelllo.so" "program/libsplx.lo" "program/libsvxlo.so" \
  "program/libswd*lo.so" "program/libtextconversiondictslx.so" \
  "program/libunopkgapp.so" "program/libvbahelperlo.so" \
  "program/libwriterfilterlo.so" "program/libwpft*lo.so" \
  "program/libvclcanvaslo.so" "program/libcairocanvaslo.so" \
  "program/libemb*lo.so" "program/libevtattlo.so" || true

# ---- B4：program 资产面 ----
run_batch b4_program_assets \
  "program/classes" "program/__pycache__" "program/intro.png" \
  "program/intro-highres.png" "program/gdbtrace" "program/java-set-classpath" \
  "program/uno_services" "program/services" 2>/dev/null || true

echo "=== 终态 ==="
du -sm "$LO"
log "final_mb=$(du_mb "$LO")"
cat "$JSONL"
