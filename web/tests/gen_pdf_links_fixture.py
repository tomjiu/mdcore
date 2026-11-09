#!/usr/bin/env python3
"""Generate web/tests/fixtures/sample_links.pdf — #466 链接导航 + #557 策略面 e2e 夹具。

与 gen_pdf_fixture.py 同构（纯标准库手写最小 PDF），差异 = 第 1 页挂
3 条 /Link 注释（三形态断言锚）+ 第 2 页挂 7 条 #557 策略探针链接：
  11/12/13 = #466 原三形态（内链 / 白名单外链 / javascript: 恶意 scheme）
  14 = 超预算原始串（2049）            → url-over-budget
  15 = 恰 2048（边界通过）              → 允许（热区在场）
  16 = %E2%80%8B 编码零宽               → zero-width-bidi-encoded
  17 = %25E2%80%8B 双重编码             → 允许（不双重解码——热区在场）
  18 = mailto:                          → 允许（#466 白名单原样）

（原始控制码形态不经夹具——pdf.js bytesToString 对 /URI 无 BOM 解码，
UTF-16BE 十六进制串会以 latin-1 面到达；raw 形态由 __pdfLinkPolicy
探针单测直测——4 例：U+200C/U+202E/U+2066/U+FEFF。）

对象布局：1=Catalog 2=Pages 3-5=Page 6-7=Font 8-10=内容流
11-13=Annot(第1页) 14-20=Annot(第2页)。
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

# ---- #557 策略探针 URL（第 2 页）----
BASE = "https://example.com/"
URL_OVER_BUDGET = BASE + "a" * (2049 - len(BASE))          # 2049 → 拒
URL_AT_BUDGET = BASE + "b" * (2048 - len(BASE))            # 恰 2048 → 允
URL_ZW_ENCODED = BASE + "%E2%80%8Bpath"                     # 编码零宽 → 拒
URL_ZW_DOUBLE = BASE + "%25E2%80%8Bpath"                    # 双重编码 → 允
URL_MAILTO = "mailto:user@example.com"                      # 白名单 → 允
# #601：mailto 附件参数——attach/insert 可携带本地文件路径交给本机邮件
# 客户端（文件外带面）→ 拒；合法 subject 混入验证「仅附件键触发」。
URL_MAILTO_ATTACH = "mailto:user@example.com?attach=/etc/passwd&subject=quote"  # 附件键 → 拒



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
    # 第 1 页挂 #466 原三形态；第 2 页挂 #557 策略七形态 + #601 附件形态
    annots_p1 = b" /Annots [11 0 R 12 0 R 13 0 R]"
    annots_p2 = b" /Annots [14 0 R 15 0 R 16 0 R 17 0 R 18 0 R 19 0 R]"
    for i, (title, extra) in enumerate(PAGE_TEXTS):
        annots = annots_p1 if i == 0 else (annots_p2 if i == 1 else b"")
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

    # ---- 链接注释（Rect = 用户空间 y 自底；同页热区互不重叠）----
    # 第 1 页（#466 原样）
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
    # 第 2 页（#557 策略七形态——矩形从上往下堆叠）
    obj(14, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 690 540 724] /Border [0 0 0] "
        + b"/A << /S /URI /URI (" + URL_OVER_BUDGET.encode("ascii") + b") >> >>"
    ))
    obj(15, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 650 540 684] /Border [0 0 0] "
        + b"/A << /S /URI /URI (" + URL_AT_BUDGET.encode("ascii") + b") >> >>"
    ))
    obj(16, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 610 540 644] /Border [0 0 0] "
        + b"/A << /S /URI /URI (" + URL_ZW_ENCODED.encode("ascii") + b") >> >>"
    ))
    obj(17, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 570 540 604] /Border [0 0 0] "
        + b"/A << /S /URI /URI (" + URL_ZW_DOUBLE.encode("ascii") + b") >> >>"
    ))
    obj(18, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 450 540 484] /Border [0 0 0] "
        b"/A << /S /URI /URI (mailto:user@example.com) >> >>"
    ))
    # #601：mailto attach 形态（Rect 在 18 与 17 之间的空档，互不重叠）
    obj(19, (
        b"<< /Type /Annot /Subtype /Link /Rect [72 495 540 529] /Border [0 0 0] "
        + b"/A << /S /URI /URI (" + URL_MAILTO_ATTACH.encode("ascii") + b") >> >>"
    ))

    n_objects = 19
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
    entries = pdf[xref_pos:].split(b"\n")[2:20]
    for num, entry in enumerate(entries[1:], start=1):
        off = int(entry.split(b" ")[0])
        assert pdf[off:off + len(f"{num} 0 obj")] == f"{num} 0 obj".encode(), \
            f"obj {num} offset {off}"
    assert b"/Dest [5 0 R" in pdf, "internal link"
    assert b"(https://example.com/mdcore-link)" in pdf, "external link"
    assert b"(javascript:alert(1))" in pdf, "malicious scheme link"
    # #557 策略形态在场
    assert URL_OVER_BUDGET.encode("ascii") in pdf, "over-budget link"
    assert URL_AT_BUDGET.encode("ascii") in pdf, "at-budget link"
    assert b"%E2%80%8Bpath" in pdf, "encoded zero-width link"
    assert b"%25E2%80%8Bpath" in pdf, "double-encoded link"
    assert b"(mailto:user@example.com)" in pdf, "mailto link"
    assert URL_MAILTO_ATTACH.encode("ascii") in pdf, "mailto attach link (#601)"
    assert pdf.rstrip().endswith(b"%%EOF"), "EOF"


def main() -> int:
    pdf = build_pdf()
    verify(pdf)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(pdf)
    print(f"ok -> {OUT} ({len(pdf)} bytes, {len(PAGE_TEXTS)} pages, 8 link annots)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
