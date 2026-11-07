#!/usr/bin/env python3
"""Generate web/tests/fixtures/sample_props.pdf — #472 文档属性 e2e 夹具。

与 gen_pdf_fixture.py 同构(纯标准库手写最小 PDF),差异 = 附加 /Info
字典(全字段断言锚:Title/Author/Subject/Keywords/Producer/Creator/
CreationDate/ModDate)——空态对照 = sample.pdf(无 /Info)。

对象布局:1=Catalog 2=Pages 3-5=Page 6-7=Font 8-10=内容流 11=/Info
(trailer /Info 11 0 R)。
"""

import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent / "fixtures" / "sample_props.pdf"

PAGE_W, PAGE_H = 612, 792
PAGE_TEXTS = [
    ("Page 1 - mdcore pdf fixture", "alpha line"),
    ("Page 2 - mdcore pdf fixture", "beta line"),
    ("Page 3 - mdcore pdf fixture", "gamma line"),
]

INFO_FIELDS = [
    ("Title", "mdcore 属性面板夹具"),
    ("Author", "张三"),
    ("Subject", "招标文件核对样张"),
    ("Keywords", "标书, 属性, 面板"),
    ("Producer", "gen_pdf_props_fixture.py"),
    ("Creator", "mdcore fixture family"),
    ("CreationDate", "D:20260101080000+08'00'"),
    ("ModDate", "D:20260102090000+08'00'"),
]


def pdf_text_string(value: str) -> bytes:
    """PDF 文本串：纯 ASCII = 字面；含非 ASCII = BOM + UTF-16BE
    （PDF 规范两种合法形态——pdf.js 对无 BOM 串按 Latin-1 解，
    UTF-8 字节直写会乱码，实测在案）。

    二进制体必须过 PDF 串转义（\\ ( ) 三字节——UTF-16BE 低字节撞上
    0x5C（如「属」= 5C 5E）会被词法层当转义符吃掉一字节，实测在案）。
    """
    def esc(raw: bytes) -> bytes:
        return raw.replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)")
    if all(ord(c) < 128 for c in value):
        return b"(" + esc(value.encode("ascii")) + b")"
    return b"(" + b"\xfe\xff" + esc(value.encode("utf-16-be")) + b")"


def content_stream(title: str, extra: str) -> bytes:
    return (
        f"BT /F1 24 Tf 72 720 Td ({title}) Tj ET\n"
        f"BT /F2 12 Tf 72 680 Td ({extra}) Tj ET\n"
    ).encode("ascii")


def build_pdf() -> bytes:
    out = bytearray()
    offsets: dict[int, int] = {}

    def obj(num: int, body: bytes) -> None:
        offsets[num] = len(out)
        out.extend(f"{num} 0 obj\n".encode("ascii"))
        out.extend(body)
        out.extend(b"\nendobj\n")

    out.extend(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")

    n_pages = len(PAGE_TEXTS)
    kids = " ".join(f"{3 + i} 0 R" for i in range(n_pages))
    obj(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    obj(2, f"<< /Type /Pages /Kids [{kids}] /Count {n_pages} >>".encode("ascii"))
    for i, (title, extra) in enumerate(PAGE_TEXTS):
        obj(3 + i, (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
            f"/Resources << /Font << /F1 6 0 R /F2 7 0 R >> >> "
            f"/Contents {8 + i} 0 R >>"
        ).encode("ascii"))
    obj(6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    obj(7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    for i, (title, extra) in enumerate(PAGE_TEXTS):
        stream = content_stream(title, extra)
        obj(8 + i, b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"endstream")

    # /Info 全字段（非 ASCII 値 = BOM + UTF-16BE 文本串——pdf.js
    # getMetadata 解回原字符串；日期走规范 D: 形态）
    info_parts = [f"/{k} ".encode("ascii") + pdf_text_string(v)
                  for k, v in INFO_FIELDS]
    obj(11, b"<< " + b" ".join(info_parts) + b" >>")

    n_objects = 11
    xref_at = len(out)
    out.extend(f"xref\n0 {n_objects + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for num in range(1, n_objects + 1):
        out.extend(f"{offsets[num]:010d} 00000 n \n".encode("ascii"))
    out.extend(
        f"trailer << /Size {n_objects + 1} /Root 1 0 R /Info 11 0 R >>\n"
        f"startxref\n{xref_at}\n%%EOF\n".encode("ascii")
    )
    return bytes(out)


def verify(pdf: bytes) -> None:
    assert pdf.startswith(b"%PDF-1.4\n"), "header"
    xref_pos = int(pdf.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    assert pdf[xref_pos:xref_pos + 4] == b"xref", "startxref -> xref"
    entries = pdf[xref_pos:].split(b"\n")[2:13]
    for num, entry in enumerate(entries[1:], start=1):
        off = int(entry.split(b" ")[0])
        assert pdf[off:off + len(f"{num} 0 obj")] == f"{num} 0 obj".encode(), \
            f"obj {num} offset {off}"
    for k, v in INFO_FIELDS:
        assert f"/{k} ".encode("ascii") + pdf_text_string(v) in pdf, f"info field {k}"
    assert b"/Info 11 0 R" in pdf, "trailer info ref"
    assert pdf.rstrip().endswith(b"%%EOF"), "EOF"


def main() -> int:
    pdf = build_pdf()
    verify(pdf)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(pdf)
    print(f"ok -> {OUT} ({len(pdf)} bytes, {len(PAGE_TEXTS)} pages, /Info x{len(INFO_FIELDS)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
