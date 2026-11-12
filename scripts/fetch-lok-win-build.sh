#!/usr/bin/env sh
# 下载 Windows LOK 内核构建产物（LO 26.2.6.3 + 118 Ely 补丁）。
#
# 用法：
#   FJT=<token> sh scripts/fetch-lok-win-build.sh [目标目录] [minimal|full]
#
# 默认 minimal（432.5 MiB，按实际使用路径裁剪，见 docs/lok-windows-build.md）。
# full = 全量 982 MiB（含 67 国词典 / Impress / Base / Python）。
#
# 云沙箱编不了 LibreOffice（20 G 磁盘 / 3.8 G 内存 / 无工具链），
# 所以内核以 release 附件形式分发。
set -eu

: "${FJT:?需要 FJT（仓库 token）}"
DEST=${1:-./_lok-win}
VARIANT=${2:-minimal}

TAG=lok-win-x64-26.2.6.3-patched
case "$VARIANT" in
  minimal)
    ASSET=instdir-min-win-x64.zip
    SHA256=F22CA7DC4AB5CA24A878D6A822BADEB629F1CF78F3688CD38054314AFF473930
    SIZE=166110126
    ROOT=instdir-min
    ;;
  full)
    ASSET=instdir-lok-win-x64-patched.zip
    SHA256=EABE5FBA39BC36E4B8A3D7C2DC8F3C33D0A9DB8E7CA92346BE0A0BE129C06A44
    SIZE=319747024
    ROOT=instdir
    ;;
  *) echo "未知变体：$VARIANT（minimal|full）" >&2; exit 2 ;;
esac

URL="https://git.mingjiu.store/sandbox/mdcore/releases/download/${TAG}/${ASSET}"

mkdir -p "$DEST"
if [ -f "$DEST/$ASSET" ] && [ "$(wc -c < "$DEST/$ASSET" | tr -d ' ')" = "$SIZE" ]; then
  echo "已存在且大小正确，跳过下载：$DEST/$ASSET"
else
  echo "下载 $ASSET（$VARIANT）…"
  # 仓库私有：不带 token 会拿到登录页 HTML，所以必须带 Authorization。
  curl -fL --retry 3 --retry-delay 2 -H "Authorization: token $FJT" "$URL" -o "$DEST/$ASSET"
fi

echo "校验 SHA256…"
echo "$SHA256  $DEST/$ASSET" | sha256sum -c -

echo "解压…"
( cd "$DEST" && unzip -q -o "$ASSET" )

echo
echo "就绪：$DEST/$ROOT"
echo "  export MDC_LOK_HOME=$DEST/$ROOT"
echo "  export MDC_LOK_LIB=$DEST/$ROOT/program/mergedlo.dll"
