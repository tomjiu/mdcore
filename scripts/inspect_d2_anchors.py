#!/usr/bin/env python3
"""解包 D2 锚点件，观察 tc 内 txbxContent 的实际 XML 形态"""
import zipfile
import re
import sys

FILES = [
    "test-artifacts/health-check/corpus/docx4j/text-extraction_extraction-probe.docx",
    "test-artifacts/health-check/corpus/docx4j/mc-fun_mc-txbx-tc-block.docx",
    "test-artifacts/health-check/corpus/docx4j/mc-fun_mc-txbx-tc-block-word365.docx",
    "test-artifacts/health-check/corpus/docx4j/text-extraction_extraction-probe.word-resave.docx",
    "test-artifacts/health-check/corpus/docx4j/text-extraction_phase0_evidence_checked-52_extraction-probe.docx",
    "test-artifacts/health-check/corpus/docx4j/text-extraction_phase0_evidence_extraction-probe-r2-word-save.docx",
    "test-artifacts/health-check/corpus/docx4j/text-extraction_phase0_evidence_extraction-probe-r3-word-save.docx",
    "test-artifacts/health-check/corpus/docx4j/text-extraction_phase0_evidence_extraction-probe-structure-word365.docx",
]

ROOT = "/home/z/my-project/repo-mdcore/"

for f in FILES:
    path = ROOT + f
    print("=" * 100)
    print(f.split("/")[-1])
    try:
        with zipfile.ZipFile(path) as z:
            xml = z.read("word/document.xml").decode("utf-8", errors="replace")
    except Exception as e:
        print("  ERROR:", e)
        continue
    # 找所有 txbxContent，输出其上下文路径（祖先栈）
    # 简化：对每个 txbxContent 出现点，往前找最近的 <w:tc 与 <w:tbl、<w:tr
    for m in re.finditer(r"<w:txbxContent>", xml):
        start = m.start()
        # 往前找包含关系：最近的开启的 w:tc / w:drawing / mc:AlternateContent / w:pict
        ctx = []
        # 检查是否在 w:drawing 内：往前最近的 <w:drawing 与 </w:drawing>
        last_drawing_open = xml.rfind("<w:drawing>", 0, start)
        last_drawing_close = xml.rfind("</w:drawing>", 0, start)
        in_drawing = last_drawing_open > last_drawing_close
        # fallback
        last_fb_open = xml.rfind("<mc:Fallback>", 0, start)
        last_fb_close = xml.rfind("</mc:Fallback>", 0, start)
        in_fallback = last_fb_open > last_fb_close
        # tc
        last_tc_open = xml.rfind("<w:tc>", 0, start)
        last_tc_close = xml.rfind("</w:tc>", 0, start)
        in_tc = last_tc_open > last_tc_close
        # tbl
        last_tbl_open = xml.rfind("<w:tbl>", 0, start)
        last_tbl_close = xml.rfind("</w:tbl>", 0, start)
        in_tbl = last_tbl_open > last_tbl_close
        # AC
        last_ac_open = xml.rfind("<mc:AlternateContent>", 0, start)
        last_ac_close = xml.rfind("</mc:AlternateContent>", 0, start)
        in_ac = last_ac_open > last_ac_close
        # 提取文本内容（前 80 字符）
        end = xml.find("</w:txbxContent>", start)
        content = xml[start:end] if end > 0 else ""
        texts = re.findall(r"<w:t[^>]*>([^<]*)</w:t>", content)
        text_preview = "".join(texts)[:60]
        print(f"  txbxContent @ {start}: in_tc={in_tc} in_drawing={in_drawing} in_fallback={in_fallback} in_ac={in_ac} in_tbl={in_tbl} text={text_preview!r}")
    # 数 tc 总数和 tbl 总数
    print(f"  [count] tc={len(re.findall(r'<w:tc>', xml))} tbl={len(re.findall(r'<w:tbl>', xml))} txbxContent={len(re.findall(r'<w:txbxContent>', xml))} AC={len(re.findall(r'<mc:AlternateContent>', xml))}")
