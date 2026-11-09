#!/usr/bin/env python3
"""gen_lok_smoke_fixture.py —— #633 LOK 冒烟夹具生成器

产出 lok_smoke_source.docx：一份最小但完整的 OOXML Word 文档，
供 mdc-lok worker 原型的「打开→渲染→编辑→保存→重开」全链冒烟使用。

内容版式（A4 纵向、1440 twips 页边距，位置可预期）：
  1. 段落一（标题）：    LOK 冒烟测试文档标题
  2. 段落二（正文锚点）： 第二段落：正文锚点词 REPLACEME 待替换。
     —— 冒烟链会用鼠标双击选中 ASCII 词 REPLACEME 后键入中文替换，
        证明键鼠输入路径 + 中文输入两件事。
  3. 2x2 表格：          单元格甲 / 单元格乙 / 单元格丙 / 单元格丁
     —— 冒烟链会点击单元格甲并键入文字（单元格编辑证据）。
  4. 内联 PNG 图片（48x48 红蓝对角块）—— 保存重开后图片仍在的证据。

用法：python3 gen_lok_smoke_fixture.py [输出路径]
默认输出到脚本所在目录的 lok_smoke_source.docx。纯标准库实现（PNG 用
zlib 手写），不依赖 PIL；产物小（<6KB）按仓内约定直接进仓。
"""

import os
import struct
import sys
import zlib


# ---------------------------------------------------------------- PNG 手写
def make_probe_png(size: int = 48) -> bytes:
    """48x48 RGBA：左上红块 + 右下蓝块 + 对角线绿，确保渲染 tile 上
    非空白像素可检测（复用 xlsx_chart 墨迹对照的『非白像素占比』思路）。"""
    width = height = size
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # 行过滤类型 0
        for x in range(width):
            if x < size // 2 and y < size // 2:
                r, g, b = 0xC0, 0x20, 0x20  # 红
            elif x >= size // 2 and y >= size // 2:
                r, g, b = 0x20, 0x20, 0xC0  # 蓝
            elif x == y or x == y + 1 or x + 1 == y:
                r, g, b = 0x20, 0x90, 0x30  # 对角绿
            else:
                r, g, b = 0xF0, 0xF0, 0xF0  # 浅底
            raw += bytes((r, g, b, 0xFF))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b""))


# ------------------------------------------------------------- OOXML 部件
CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>
"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"""

DOC_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rIdImg1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
</Relationships>
"""


def para(text: str) -> str:
    return ('<w:p><w:pPr><w:rPr><w:rFonts w:ascii="Liberation Serif"'
            ' w:hAnsi="Liberation Serif" w:eastAsia="Noto Sans CJK SC"/>'
            '</w:rPr></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Liberation Serif"'
            ' w:hAnsi="Liberation Serif" w:eastAsia="Noto Sans CJK SC"/>'
            f'</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


def cell(text: str) -> str:
    return ('<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>'
            + para(text) + "</w:tc>")


def document_xml() -> str:
    table = ('<w:tbl><w:tblPr><w:tblW w:w="9026" w:type="dxa"/>'
             '<w:tblBorders>'
             '<w:top w:val="single" w:sz="4" w:color="444444"/>'
             '<w:left w:val="single" w:sz="4" w:color="444444"/>'
             '<w:bottom w:val="single" w:sz="4" w:color="444444"/>'
             '<w:right w:val="single" w:sz="4" w:color="444444"/>'
             '</w:tblBorders></w:tblPr>'
             '<w:tblGrid><w:gridCol w:w="4513"/><w:gridCol w:w="4513"/></w:tblGrid>'
             '<w:tr>' + cell("单元格甲") + cell("单元格乙") + "</w:tr>"
             '<w:tr>' + cell("单元格丙") + cell("单元格丁") + "</w:tr>"
             "</w:tbl>")
    # 内联图片：48x48 像素，1px=9525EMU（96dpi）→ 457200 EMU
    drawing = ('<w:p><w:r><w:drawing>'
               '<wp:inline distT="0" distB="0" distL="0" distR="0">'
               '<wp:extent cx="457200" cy="457200"/>'
               '<wp:effectExtent l="0" t="0" r="0" b="0"/>'
               '<wp:docPr id="1" name="图片 1"/>'
               '<wp:cNvGraphicFramePr>'
               '<a:graphicFrameLocks noChangeAspect="1"/>'
               "</wp:cNvGraphicFramePr>"
               '<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
               '<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
               '<pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">'
               "<pic:nvPicPr>"
               '<pic:cNvPr id="1" name="image1.png"/>'
               "<pic:cNvPicPr/>"
               "</pic:nvPicPr>"
               '<pic:blipFill><a:blip r:embed="rIdImg1"/>'
               "<a:stretch><a:fillRect/></a:stretch></pic:blipFill>"
               "<pic:spPr>"
               '<a:xfrm><a:off x="0" y="0"/><a:ext cx="457200" cy="457200"/></a:xfrm>'
               '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>'
               "</pic:spPr>"
               "</pic:pic></a:graphicData></a:graphic></wp:inline>"
               "</w:drawing></w:r></w:p>")
    body = (para("LOK 冒烟测试文档标题")
            + para("第二段落：正文锚点词 REPLACEME 待替换。")
            + table
            + drawing)
    sect = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>'
            '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>'
            "</w:sectPr>")
    return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
            '<w:document'
            ' xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
            ' xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
            ' xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"'
            ' xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
            "<w:body>" + body + sect + "</w:body></w:document>")


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "lok_smoke_source.docx")
    import zipfile
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES)
        z.writestr("_rels/.rels", ROOT_RELS)
        z.writestr("word/document.xml", document_xml())
        z.writestr("word/_rels/document.xml.rels", DOC_RELS)
        z.writestr("word/media/image1.png", make_probe_png())
    print(f"wrote {out} ({os.path.getsize(out)} bytes)")


if __name__ == "__main__":
    main()
