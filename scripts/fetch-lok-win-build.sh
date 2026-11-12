#!/usr/bin/env sh
# 下载 Windows LOK 内核构建产物（LO 26.2.6.3 + 118 Ely 补丁）。
#
# 用法：
#   FJT=<token> sh scripts/fetch-lok-win-build.sh [目标目录]
#
# 云沙箱编不了 LibreOffice（20 G 磁盘 / 3.8 G 内存 / 无工具链），
# 所以内核以 release 附件形式分发。详见 docs/lok-windows-build.md。
set -eu

: "${FJT:?需要 FJT（仓库 token）}"
DEST=${1:-./_lok-win}

TAG=lok-win-x64-26.2.6.3-patched
ASSET=instdir-lok-win-x64-patched.zip
URL="https://git.mingjiu.store/sandbox/mdcore/releases/download/${TAG}/${ASSET}"
SHA256=EABE5FBA39BC36E4B8A3D7C2DC8F3C33D0A9DB8E7CA92346BE0A0BE129C06A44
SIZE=319747024

mkdir -p "$DEST"
if [ -f "$DEST/$ASSET" ] && [ "$(wc -c < "$DEST/$ASSET" | tr -d ' ')" = "$SIZE" ]; then
  echo "已存在且大小正确，跳过下载：$DEST/$ASSET"
else
  echo "下载 $ASSET（305 MiB）…"
  # 仓库私有：不带 token 会拿到登录页 HTML，所以必须带 Authorization。
  curl -fL --retry 3 --retry-delay 2 -H "Authorization: token $FJT" "$URL" -o "$DEST/$ASSET"
fi

echo "校验 SHA256…"
echo "$SHA256  $DEST/$ASSET" | sha256sum -c -

echo "解压…"
( cd "$DEST" && unzip -q -o "$ASSET" )

echo
echo "就绪：$DEST/instdir"
echo "  export MDC_LOK_HOME=$DEST/instdir"
echo "  export MDC_LOK_LIB=$DEST/instdir/program/mergedlo.dll"
