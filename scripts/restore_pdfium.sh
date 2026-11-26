#!/usr/bin/env bash
# pdfium 资产恢复（沙箱回收清 bin/lib 后用）——#307 转换质量门 + #68 读面依赖。
# 测试定位序第二位: crates/pdf/../../bin/lib/libpdfium.so（仓根 bin/lib/）。
set -euo pipefail
DEST="$(cd "$(dirname "$0")/.." && pwd)/bin/lib/libpdfium.so"
if [ -f "$DEST" ]; then echo "已在位: $DEST"; exit 0; fi
URL="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz"
TMP=$(mktemp -d)
curl -sL -o "$TMP/pdfium.tgz" "$URL"
mkdir "$TMP/x"
tar xzf "$TMP/pdfium.tgz" -C "$TMP/x"
mkdir -p "$(dirname "$DEST")"
cp "$TMP/x/lib/libpdfium.so" "$DEST"
rm -rf "$TMP"
echo "恢复完成: $DEST ($(stat -c%s "$DEST") bytes)"
