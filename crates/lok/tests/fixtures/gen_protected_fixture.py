#!/usr/bin/env python3 -I
"""#50 C5 批3：保护域 fixture 生成器（幂等——已在 fixtures/ 的产物由此再生）。

从 smoke-chars-img-table.docx 注入 `w:documentProtection`（OOXML 规范面真锁，
非模拟）到 settings.xml：

- protected-readonly.docx —— w:edit="readOnly" w:enforcement="1"（整文档只读锁）
- protected-forms.docx   —— w:edit="forms" w:enforcement="1"（只允许填表单域）

用法：cd crates/lok/tests/fixtures && python3 -I gen_protected_fixture.py
"""
import zipfile

BASE = "smoke-chars-img-table.docx"


def make(dst: str, mode: str) -> None:
    zin = zipfile.ZipFile(BASE)
    zout = zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED)
    for item in zin.namelist():
        data = zin.read(item)
        if item == "word/settings.xml":
            xml = data.decode("utf-8")
            ins = f'<w:documentProtection w:edit="{mode}" w:enforcement="1"/>'
            assert "</w:settings>" in xml, "settings.xml 结构异常"
            xml = xml.replace("</w:settings>", ins + "</w:settings>")
            data = xml.encode("utf-8")
        zout.writestr(item, data)
    zout.close()
    print(f"{dst} 生成（edit={mode}）")


if __name__ == "__main__":
    make("protected-readonly.docx", "readOnly")
    make("protected-forms.docx", "forms")
