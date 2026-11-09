#!/usr/bin/env python3
"""生成 LOK smoke 测试夹具 DOCX（#633）。

产物：smoke-chars-img-table.docx —— 中文正文（含可搜索锚文本）+ 相邻段落 +
2x2 表格（含唯一单元格文本）+ 内嵌 PNG 图片。
转换器：LibreOffice headless（HTML -> DOCX）。
可重复执行，产物字节由转换器决定（以锚文本/表格/图片三要素校验为准）。
"""
import base64
import pathlib
import struct
import subprocess
import sys
import zlib

HERE = pathlib.Path(__file__).resolve().parent

# ---- 手工构造 24x24 双色 PNG（红/蓝棋盘 + 白角标），零三方依赖 ----
W = H = 24


def chunk(tag: bytes, data: bytes) -> bytes:
    return (struct.pack(">I", len(data)) + tag + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))


rows = bytearray()
for y in range(H):
    rows.append(0)  # filter: none
    for x in range(W):
        if x < 4 and y < 4:
            rows += bytes((255, 255, 255))          # 左上白角标
        elif (x // 4 + y // 4) % 2 == 0:
            rows += bytes((200, 30, 30))            # 红
        else:
            rows += bytes((30, 40, 190))            # 蓝
png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
       + chunk(b"IEND", b""))
(HERE / "probe.png").write_bytes(png)

# Flat ODF（.fodt）源：图片经 office:binary-data 内嵌；Writer 加载为普通文本文档
# （HTML 会被 Writer/Web 过滤器加载，Web 文档没有 DOCX 导出过滤——实测教训）。
PNG_B64 = base64.b64encode(png).decode()

FODT = f"""<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
 office:version="1.2" office:mimetype="application/vnd.oasis.opendocument.text">
<office:body>
<office:text>
<text:p text:style-name="Standard">锚点正文句甲：这是 LOK 冒烟夹具的第一段中文正文，用于搜索定位与键盘替换编辑。</text:p>
<text:p text:style-name="Standard">相邻段落乙：本段与锚点段相邻，保存重开后必须原样保留，用于校验相邻内容不丢。</text:p>
<text:p text:style-name="Standard">图片段落：<draw:frame draw:name="probe" svg:width="2cm" svg:height="2cm" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"><draw:image><office:binary-data>{PNG_B64}</office:binary-data></draw:image></draw:frame></text:p>
<text:p text:style-name="Standard">表格前置段落：下表用于校验单元格键盘编辑。</text:p>
<table:table table:name="probetable">
<table:table-column/><table:table-column/>
<table:table-row><table:table-cell><text:p>表头一</text:p></table:table-cell><table:table-cell><text:p>表头二</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell><text:p>表格单元格丙</text:p></table:table-cell><table:table-cell><text:p>表格单元格丁</text:p></table:table-cell></table:table-row>
</table:table>
<text:p text:style-name="Standard">末尾段落戊：文档结尾哨兵文本。</text:p>
</office:text>
</office:body>
</office:document>
"""
(HERE / "fixture-src.fodt").write_text(FODT, encoding="utf-8")


def main() -> int:
    out = HERE / "smoke-chars-img-table.docx"
    if out.exists():
        out.unlink()
    # 独立 user profile：残留 soffice / 默认 profile 锁会让 headless 转换静默不产出
    # （实测教训；与仓内 office-io convert.rs 的 -env:UserInstallation 纪律同源）。
    profile = pathlib.Path("/tmp") / f"mdc-lok-gen-profile-{__import__('os').getpid()}"
    r = subprocess.run(
        ["soffice", "--headless",
         f"-env:UserInstallation=file://{profile}",
         "--convert-to", "docx",
         str(HERE / "fixture-src.fodt"), "--outdir", str(HERE)],
        capture_output=True, text=True, timeout=180,
    )
    raw = HERE / "fixture-src.docx"  # soffice 输出名跟随源文件名
    if r.returncode != 0 or not raw.exists():
        print("[gen] soffice 转换失败:", r.returncode, r.stderr[:400])
        return 1
    raw.rename(out)
    # 校验三要素（zip 层面）：锚文本 / 表格 / 图片 media
    import zipfile
    with zipfile.ZipFile(out) as z:
        doc = z.read("word/document.xml").decode("utf-8", "replace")
        names = z.namelist()
    checks = {
        "锚文本(甲)": "锚点正文句甲" in doc,
        "相邻段(乙)": "相邻段落乙" in doc,
        "表格": "<w:tbl>" in doc,
        "单元格(丙)": "表格单元格丙" in doc,
        "末尾(戊)": "末尾段落戊" in doc,
        "图片media": any(n.startswith("word/media/") for n in names),
        "图片引用": "graphic" in doc or "pic:pic" in doc or "w:drawing" in doc,
    }
    bad = [k for k, v in checks.items() if not v]
    for k, v in checks.items():
        print(f"[gen] {'OK ' if v else 'FAIL'} {k}")
    print(f"[gen] 产物: {out} ({out.stat().st_size} bytes)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
