#!/usr/bin/env bash
# LO 裁剪实验门（2026-11-12 重做轮；mdcore 任务书实测缺口回填）。
#
# 用途：对一个【LO 安装树副本】跑 mdcore 的真实 LOK 消费面作为「不破」门：
#   1) lok_smoke 全链（documentLoad→initializeForRendering→getDocumentSize→
#      paintTile 非空像素→postKeyEvent/postUnoCommand→回调脏态→saveAs docx→
#      重开校验→双 worker 并行→超时看门狗）
#   2) export_pdf 面（全文 3 页 + 页范围 2-3 + 单页；PDF 头/页数校验在
#      测试断言内）
#
# 用法：lo_trim_gate.sh <LO 安装根副本> <工作目录>
#   MDC_LOK_HOME=<副本> 后跑两个测试；两者全绿 → exit 0。
#   副本须含 program/（fundamentalrc 里 URE_BOOTSTRAP 等相对引用按安装根
#   解析，副本整树复制即可自洽）。
#
# 设计依据：mdcore 对 LO 的真实消费面只有 (a) LOK 编辑渲染链 (b) saveAs
# 导出链（含 PDF）。smoke 链覆盖 (a)(b) 的 docx 面，export_pdf 测试覆盖
# PDF 面——两门全绿即「裁剪未破消费面」的实证口径（沙箱可复跑）。
set -euo pipefail

LO_ROOT="${1:?usage: lo_trim_gate.sh <LO 安装根副本> <工作目录>}"
WORK="${2:?usage: lo_trim_gate.sh <LO 安装根副本> <工作目录>}"

[ -d "$LO_ROOT/program" ] || { echo "[gate] FATAL: $LO_ROOT/program 不存在"; exit 2; }
[ -f "$LO_ROOT/program/libmergedlo.so" ] || { echo "[gate] FATAL: 内核 libmergedlo.so 缺失"; exit 2; }

export PATH="/home/z/.cargo/bin:$PATH"
export MDC_LOK_HOME="$LO_ROOT"
export MDC_LOK_WORKER_BIN="${MDC_LOK_WORKER_BIN:-}"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
cd /home/z/my-project/mdcore

SMOKE_LOG="$WORK/gate-smoke.log"
PDF_LOG="$WORK/gate-pdf.log"

# 门 1：lok_smoke 全链（SKIP 视为失败——副本在场时 discover 必须成功；
# SKIP 横幅说明副本树断了，宁可红不可假绿）
cargo test -p mdc-lok --test lok_smoke -- --nocapture >"$SMOKE_LOG" 2>&1 || true
tail -2 "$SMOKE_LOG"
grep -q "SKIP lok_smoke" "$SMOKE_LOG" && { echo "[gate] FAIL: smoke SKIP（副本 discover 失败）"; exit 1; }
grep -q "test result: ok. 1 passed" "$SMOKE_LOG" || { echo "[gate] FAIL: smoke 未全绿（详见 $SMOKE_LOG）"; exit 1; }

# 门 2：export_pdf 契约（含页范围与单页断言）
cargo test -p mdc-lok --test export_pdf -- --nocapture >"$PDF_LOG" 2>&1 || true
tail -2 "$PDF_LOG"
grep -q "SKIP" "$PDF_LOG" && { echo "[gate] FAIL: export_pdf SKIP"; exit 1; }
grep -q "test result: ok. 1 passed" "$PDF_LOG" || { echo "[gate] FAIL: export_pdf 未全绿"; exit 1; }

echo "[gate] PASS: $LO_ROOT（smoke 全链 + export_pdf 两门全绿）"
