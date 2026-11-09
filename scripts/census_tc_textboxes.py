#!/usr/bin/env python3
"""630 件全语料普查：tc 内 txbxContent 形态分布（D2 修复设计依据）"""
import zipfile
import re
import os

ROOT = "/home/z/my-project/repo-mdcore/test-artifacts/health-check/corpus"

def ctx_flags(xml, pos):
    def inside(open_t, close_t):
        return xml.rfind(open_t, 0, pos) > xml.rfind(close_t, 0, pos)
    return {
        "tc": inside("<w:tc>", "</w:tc>"),
        "drawing": inside("<w:drawing>", "</w:drawing>"),
        "fallback": inside("<mc:Fallback>", "</mc:Fallback>"),
        "sdt": inside("<w:sdt>", "</w:sdt>") or inside("<w:sdtContent>", "</w:sdtContent>"),
    }

stats = {
    "files_with_tc_boxes": 0,
    "tc_drawing_boxes": 0,     # D2 目标形态（drawing 内，非 fallback）
    "tc_vml_boxes": 0,         # 非 drawing 非 fallback（既有 VML 趟已取——不得双取）
    "tc_fallback_boxes": 0,    # fallback 副本（GT 不计，不得取）
    "tc_sdt_boxes": 0,         # body 级 sdt 内（GT 计 in_sdt——不得取）
}
files_detail = []
for dirpath, _, filenames in os.walk(ROOT):
    for fn in sorted(filenames):
        if not fn.endswith(".docx"):
            continue
        path = os.path.join(dirpath, fn)
        rel = os.path.relpath(path, ROOT)
        try:
            with zipfile.ZipFile(path) as z:
                # 主件名（默认 document.xml；非常规名忽略——GT 侧有 main_part_name 逻辑，这里简化）
                names = z.namelist()
                main = "word/document.xml" if "word/document.xml" in names else None
                if main is None:
                    continue
                xml = z.read(main).decode("utf-8", errors="replace")
        except Exception:
            continue
        tc_boxes = []
        for m in re.finditer(r"<w:txbxContent[ >]", xml):
            f = ctx_flags(xml, m.start())
            if f["tc"]:
                tc_boxes.append(f)
        if not tc_boxes:
            continue
        stats["files_with_tc_boxes"] += 1
        detail = {"rel": rel, "boxes": []}
        for f in tc_boxes:
            if f["fallback"]:
                stats["tc_fallback_boxes"] += 1
                kind = "fallback"
            elif f["sdt"] and not f["tc"] is None:
                # sdt 内（rfind 启发式：sdt 开标签最近且未闭合）
                stats["tc_sdt_boxes"] += 1
                kind = "sdt"
            elif f["drawing"]:
                stats["tc_drawing_boxes"] += 1
                kind = "drawing"
            else:
                stats["tc_vml_boxes"] += 1
                kind = "vml"
            detail["boxes"].append(kind)
        files_detail.append(detail)

print(stats)
print()
for d in files_detail:
    print(f"  {d['rel']}: {d['boxes']}")
