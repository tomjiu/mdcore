#!/usr/bin/env python3
"""Generate web/tests/fixtures/sample_outline.pdf — #463 大纲树 e2e 夹具。

与 gen_pdf_fixture.py 同构（纯标准库手写最小 PDF），差异 = 附加 /Outlines
树（3 节点：Chapter 1 →(子) Section 1.1 →(兄弟) Chapter 2），供
pdf_outline.py 断言树渲染/点击跳页/空态对照。

对象布局（1 基编号，xref 全 14 项 + 哨兵）：
  1=Catalog(/Outlines 11) 2=Pages 3-5=Page 6-7=Font 8-10=内容流
  11=Outlines 根(/First 12 /Last 14 /Count 3)
  12=Chapter 1(/Dest [3 0 R /XYZ] /First 13 /Count 1 /Parent 11)
  13=Section 1.1(/Dest [4 0 R /XYZ] /Parent 12)
  14=Chapter 2(/Dest [5 0 R /XYZ] /Prev 12 /Parent 11)

页文本与 sample.pdf 同约定（"Page N - mdcore pdf fixture"——跳页断言
可复用页码跟踪锚）。
"""

import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent / "fixtures" / "sample_outline.pdf"

PAGE_W, PAGE_H = 612, 792
PAGE_TEXTS = [
    ("Page 1 - mdcore pdf fixture", "alpha line"),
    ("Page 2 - mdcore pdf fixture", "beta line"),
    ("Page 3 - mdcore pdf fixture", "gamma line"),
]


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
    obj(1, b"<< /Type /Catalog /Pages 2 0 R /Outlines 11 0 R >>")
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

    # /Outlines 树（3 节点——pdf.js getOutline 的 /Title /Dest /First /
    # /Last /Parent /Prev /Count 面全走显式数组目标，无命名目标）。
    obj(11, b"<< /Type /Outlines /First 12 0 R /Last 14 0 R /Count 3 >>")
    obj(12, (
        b"<< /Title (Chapter 1) /Parent 11 0 R /Dest [3 0 R /XYZ null null null] "
        b"/First 13 0 R /Last 13 0 R /Next 14 0 R /Count 1 >>"
    ))
    obj(13, (
        b"<< /Title (Section 1.1) /Parent 12 0 R /Dest [4 0 R /XYZ null null null] >>"
    ))
    obj(14, (
        b"<< /Title (Chapter 2) /Parent 11 0 R /Prev 12 0 R "
        b"/Dest [5 0 R /XYZ null null null] >>"
    ))

    n_objects = 14
    xref_at = len(out)
    out.extend(f"xref\n0 {n_objects + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for num in range(1, n_objects + 1):
        out.extend(f"{offsets[num]:010d} 00000 n \n".encode("ascii"))
    out.extend(
        f"trailer << /Size {n_objects + 1} /Root 1 0 R >>\n"
        f"startxref\n{xref_at}\n%%EOF\n".encode("ascii")
    )
    return bytes(out)


def verify(pdf: bytes) -> None:
    assert pdf.startswith(b"%PDF-1.4\n"), "header"
    xref_pos = int(pdf.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    assert pdf[xref_pos:xref_pos + 4] == b"xref", "startxref -> xref"
    entries = pdf[xref_pos:].split(b"\n")[2:16]
    for num, entry in enumerate(entries[1:], start=1):
        off = int(entry.split(b" ")[0])
        assert pdf[off:off + len(f"{num} 0 obj")] == f"{num} 0 obj".encode(), \
            f"obj {num} offset {off}"
    assert b"/Title (Chapter 1)" in pdf, "outline title 1"
    assert b"/Title (Section 1.1)" in pdf, "outline title 2"
    assert b"/Title (Chapter 2)" in pdf, "outline title 3"
    assert b"/Next 14 0 R" in pdf, "sibling chain (/Next)——pdf.js 走 First→Next 链"
    assert pdf.rstrip().endswith(b"%%EOF"), "EOF"


def main() -> int:
    pdf = build_pdf()
    verify(pdf)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(pdf)
    print(f"ok -> {OUT} ({len(pdf)} bytes, {len(PAGE_TEXTS)} pages, 3 outline nodes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
