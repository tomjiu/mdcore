#!/usr/bin/env python3
"""Generate web/tests/fixtures/sample_links.pdf — #466 链接导航 e2e 夹具。

与 gen_pdf_fixture.py 同构（纯标准库手写最小 PDF），差异 = 第 1 页挂
3 条 /Link 注释（三形态断言锚）：
  12 = 内链 /Dest [5 0 R /XYZ ...]（→ 第 3 页）
  13 = 外链 /A /URI (https://example.com/mdcore-link)（白名单 scheme）
  14 = 恶意 /A /URI (javascript:alert(1))（非白名单——不渲染断言锚）

对象布局：1=Catalog 2=Pages 3-5=Page 6-7=Font 8-10=内容流 12-14=Annot。
"""

import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent / "fixtures" / "sample_links.pdf"

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
    # 第 1 页挂 /Annots（三条 Link——内链/白名单外链/恶意 scheme；
    # 对象 11-13 连续编号，n_objects = 13）
    obj(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    obj(2, f"<< /Type /Pages /Kids [{kids}] /Count {n_pages} >>".encode("ascii"))
    for i, (title, extra) in enumerate(PAGE_TEXTS):
        annots = b" /Annots [11 0 R 12 0 R 13 0 R]" if i == 0 else b""
        obj(3 + i, (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
            f"/Resources << /Font << /F1 6 0 R /F2 7 0 R >> >> "
            f"/Contents {8 + i} 0 R{annots.decode()} >>"
        ).encode("ascii"))
    obj(6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    obj(7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    for i, (title, extra) in enumerate(PAGE_TEXTS):
        stream = content_stream(title, extra)
        obj(8 + i, b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"endstream")

    # 链接注释（Rect = 用户空间 y 自底；三条热区互不重叠）
    obj(11, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 700 420 740] "
        b"/Border [0 0 0] /Dest [5 0 R /XYZ null null null] >>"
    ))
    obj(12, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 660 420 696] /Border [0 0 0] "
        b"/A << /S /URI /URI (https://example.com/mdcore-link) >> >>"
    ))
    obj(13, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 620 420 656] /Border [0 0 0] "
        b"/A << /S /URI /URI (javascript:alert(1)) >> >>"
    ))

    n_objects = 13
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
    entries = pdf[xref_pos:].split(b"\n")[2:15]
    for num, entry in enumerate(entries[1:], start=1):
        off = int(entry.split(b" ")[0])
        assert pdf[off:off + len(f"{num} 0 obj")] == f"{num} 0 obj".encode(), \
            f"obj {num} offset {off}"
    assert b"/Dest [5 0 R" in pdf, "internal link"
    assert b"(https://example.com/mdcore-link)" in pdf, "external link"
    assert b"(javascript:alert(1))" in pdf, "malicious scheme link"
    assert pdf.rstrip().endswith(b"%%EOF"), "EOF"


def main() -> int:
    pdf = build_pdf()
    verify(pdf)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(pdf)
    print(f"ok -> {OUT} ({len(pdf)} bytes, {len(PAGE_TEXTS)} pages, 3 link annots)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
