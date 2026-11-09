#!/usr/bin/env python3
"""检查锚点件宿主表单元格文本 + v1-rejected 容器状态"""
import zipfile
import re

ROOT = "/home/z/my-project/repo-mdcore/test-artifacts/health-check/corpus/"

FILES = [
    "docx4j/text-extraction_extraction-probe.docx",
    "docx4j/mc-fun_mc-txbx-tc-block.docx",
    "docx4j/mc-fun_mc-txbx-tc-block-word365.docx",
    "docx4j/text-extraction_phase0_evidence_extraction-probe-v1-rejected.docx",
    "docx4j/text-extraction_phase0_extraction-probe.docx",
]

for rel in FILES:
    print("=" * 90)
    print(rel)
    try:
        with zipfile.ZipFile(ROOT + rel) as z:
            names = z.namelist()
            # v1-rejected: 检查 zip 完整性
            bad = z.testzip()
            print(f"  zip entries={len(names)} badfile={bad}")
            xml = z.read("word/document.xml").decode("utf-8", errors="replace")
    except Exception as e:
        print("  ERROR:", e)
        continue
    # 找包含 TBX25_IN_CELL 或 CELL_CHOICE_BOX 的 tc，输出该 tc 的全部 w:t
    for marker in ["TBX25_IN_CELL", "CELL_CHOICE_BOX"]:
        idx = xml.find(marker)
        if idx < 0:
            continue
        # 找包含此位置的 w:tc
        tc_start = xml.rfind("<w:tc>", 0, idx)
        tc_end = xml.find("</w:tc>", idx)
        tc_xml = xml[tc_start : tc_end + 8]
        texts = re.findall(r"<w:t[^>]*>([^<]*)</w:t>", tc_xml)
        print(f"  [{marker}] 宿主 tc 文本: {texts}")
        # 该 tc 所在的 w:tr 的行内其他 tc 文本
        tr_start = xml.rfind("<w:tr", 0, tc_start)
        tr_end = xml.find("</w:tr>", tc_start)
        tr_xml = xml[tr_start:tr_end]
        tcs = re.findall(r"<w:tc>.*?</w:tc>", tr_xml, re.DOTALL)
        for i, tc in enumerate(tcs):
            tt = re.findall(r"<w:t[^>]*>([^<]*)</w:t>", tc)
            print(f"    行内 tc[{i}]: {tt}")
