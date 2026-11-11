"""#143（跟进件）—— NSIS per-user（HKCU）+ 礼貌类型注册：行为结构锁。

钉住 PR #150（Agent-m3q7 主件）+ 跟进件（Agent-iicv「默认应用」面）合计
的行为面。三层钉子（无 CI 平台的本地门）：

  ① 模板结构断言（本文件主体）——注册表面在源码层不可回退；
  ② 渲染产物断言——真实 builder 走查后对 `installer.nsi` 复验（防占位符
    替换/构建器改动破坏结构）；
  ③ makensis 编译面（可得时执行）——Debian nsis 3.11 / choco nsis 3.x
    （BUILD-VERIFY §3 口径）；不可得时 SKIP（如实报告，不伪装通过）。

Windows 注册表实机行为（预装 .md=别的程序 → 安装后不动 等）归 R24-d
真机窗口（checklist D2a 预装关联验证项，跟进件一并登记）。

双平台可跑：纯 Python 文本分析；Linux 与 Windows CI 语义一致。

运行（仓根，直接路径——`packaging` 与 stdlib 模块撞名不可 -m 导入）：
    python3 packaging/tests/test_ns_polite_registration.py -v
"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(os.environ.get("PACKAGING_UNDER_TEST", Path(__file__).resolve().parents[2]))
TPL = ROOT / "packaging" / "nsis" / "installer.nsi.tpl"


def tpl_text() -> str:
    return TPL.read_text(encoding="utf-8")


def strip_comments(text: str) -> str:
    """剥离 NSIS 注释行（`;`/`#` 开头的行）。

    注册表面断言只作用于**指令面**——历史注释允许提及 HKLM/HKCR
    等缺陷根（「为什么改」的记录价值高于字面禁忌）。本模板注释
    均为整行制，整行剥离即零误伤。
    """
    return "\n".join(
        line for line in text.splitlines()
        if not line.lstrip().startswith((";", "#"))
    )


class PerUserSurfaceTests(unittest.TestCase):
    """承载面：全程 HKCU、免管理员、per-user 落位。"""

    @classmethod
    def setUpClass(cls):
        assert TPL.is_file(), f"模板缺失：{TPL}"
        cls.text = strip_comments(tpl_text())

    def test_no_hklm_machine_level_writes(self):
        self.assertNotIn("HKLM", self.text, "HKLM 机器级写入必须清零（#143 per-user 面）")

    def test_request_execution_level_user(self):
        self.assertIn("RequestExecutionLevel user", self.text)
        self.assertNotIn("RequestExecutionLevel admin", self.text)
        self.assertNotIn("RequestExecutionLevel none", self.text)

    def test_install_dir_is_per_user_location(self):
        self.assertIn('InstallDir "$LOCALAPPDATA\\Programs\\${APPEXEC}"', self.text)
        self.assertNotIn("PROGRAMFILES64", self.text)
        self.assertIn('InstallDirRegKey HKCU "Software\\${APPID}" "InstallDir"', self.text)

    def test_shell_context_current(self):
        self.assertEqual(self.text.count("SetShellVarContext current"), 2,
                         "安装与卸载两段各须显式钉 current")


class PoliteRegistrationTests(unittest.TestCase):
    """礼貌关联：只在无主（或悬空/已归自己）时设默认与「新建」；
    「打开方式」面登记而不动默认；「默认应用」面列表化不接管。"""

    @classmethod
    def setUpClass(cls):
        assert TPL.is_file(), f"模板缺失：{TPL}"
        cls.text = strip_comments(tpl_text())

    def test_reads_existing_md_handler_before_writes(self):
        read_at = self.text.index('ReadRegStr $Tmp0 HKCR ".md" ""')
        write_at = self.text.index('!insertmacro TypePut "${CLASSES}\\.md" ""')
        self.assertLess(read_at, write_at, "先读后写次序是礼貌注册的地基")

    def test_takeover_guard_covers_dangling_and_self(self):
        """无主三分支：空 / 悬空（EnumRegKey 失败）/ 已归自己。"""
        self.assertIn('ReadRegStr $Tmp0 HKCR ".md" ""', self.text)
        self.assertIn('EnumRegKey $Tmp1 HKCR "$Tmp0" 0', self.text,
                      "悬空 progid（默认值指向已删类）必须判为无主")
        self.assertIn('${OrIf} $Tmp0 == "${PROGID}"', self.text,
                      "已归自己 → 幂等重写面")

    def test_shellnew_only_in_takeover_branch(self):
        """「新建」菜单（ShellNew）只在无主接管分支内。"""
        guard = self.text.index('${OrIf} $Tmp0 == "${PROGID}"')
        takeover = self.text.index('!insertmacro TypePut "${CLASSES}\\.md" ""')
        shellnew = self.text.index('!insertmacro TypePut "${CLASSES}\\.md\\ShellNew"')
        self.assertLess(guard, takeover, "接管写在守卫之后")
        self.assertLess(takeover, shellnew, "ShellNew 跟随默认值写入（同一分支）")

    def test_openwith_face_writes_only_own_value_name(self):
        """「打开方式」面：OpenWithProgids 值名 = 自有 progid（他应用登记不相扰）。"""
        self.assertIn('!insertmacro TypePut "${CLASSES}\\.md\\OpenWithProgids" "${PROGID}" ""', self.text)
        self.assertIn('"FriendlyAppName" "${APPNAME}"', self.text)

    def test_default_apps_listing_without_takeover(self):
        """「默认应用」面（跟进件）：RegisteredApplications + Capabilities
        列表化登记，不接管默认——用户从系统设置自选。"""
        self.assertIn(
            '!insertmacro TypePut "Software\\RegisteredApplications" "${APPNAME}"',
            self.text, "RegisteredApplications 登记必须在")
        self.assertIn(
            '!insertmacro TypePut "Software\\${APPID}\\Capabilities\\FileAssociations" ".md" "${PROGID}"',
            self.text, "Capabilities/FileAssociations 必须在（.md → 自有 progid）")
        self.assertIn('"ApplicationName" "${APPNAME}"', self.text)

    def test_write_failure_names_key_and_aborts(self):
        """写不进即安装失败：逐写记名（$TypeFailKey）→ 弹窗 → Abort。"""
        self.assertIn("Var TypeFailKey", self.text)
        self.assertIn("StrCpy $TypeFailKey", self.text)
        self.assertIn('${If} $TypeFailKey != ""', self.text)
        m = re.search(r'\$\{If\} \$TypeFailKey != ""\n(.*?)\$\{EndIf\}', self.text, re.S)
        self.assertIsNotNone(m, "失败闸分支缺失")
        self.assertIn("Abort", m.group(1), "失败路径必须 Abort（不静默继续）")
        self.assertIn("$TypeFailKey", m.group(1), "弹窗必须报出首败键名（blame 语义）")

    def test_explorer_notified_after_type_changes(self):
        """SHChangeNotify(SHCNE_ASSOCCHANGED) ×2（安装/卸载各一）。"""
        self.assertEqual(self.text.count("SHChangeNotify"), 2)


class PoliteUninstallTests(unittest.TestCase):
    """卸载所有权：换目录后旧卸载不删新的；只清自己写入的面。"""

    @classmethod
    def setUpClass(cls):
        assert TPL.is_file(), f"模板缺失：{TPL}"
        cls.un = strip_comments(
            tpl_text()[tpl_text().index('Section "Uninstall"'):])

    def test_no_hkcr_deletes(self):
        self.assertNotIn("HKCR", self.un)

    def test_type_face_gated_by_open_command_ownership(self):
        """类型面清理以「open 命令仍指向本安装目录」为闸（换目录后旧卸载不动新登记）。"""
        self.assertIn(
            'ReadRegStr $Tmp0 HKCU "${CLASSES}\\${PROGID}\\shell\\open\\command" ""',
            self.un)
        self.assertIn("${If} $Tmp0 == $Cmd", self.un)
        self.assertLess(self.un.index("${If} $Tmp0 == $Cmd"),
                        self.un.index('DeleteRegKey HKCU "${CLASSES}\\${PROGID}"'),
                        "先所有权闸后删 progid")

    def test_md_default_cleanup_gated_by_progid(self):
        """.md 默认值/「新建」仅当仍归自己才清（用户改选别的默认 → 不动）。"""
        self.assertIn('${If} $Tmp0 == "${PROGID}"', self.un)
        self.assertIn('DeleteRegValue HKCU "${CLASSES}\\.md\\ShellNew" "NullFile"', self.un)

    def test_uninstall_deletes_files_not_directory_tree(self):
        for rel in ('Delete "$INSTDIR\\${APPEXEC}.exe"',
                    'Delete "$INSTDIR\\pdfium.dll"',
                    'Delete "$INSTDIR\\uninstall.exe"',
                    'RMDir "$INSTDIR"'):
            self.assertIn(rel, self.un)
        self.assertNotIn('RMDir /r "$INSTDIR"', self.un,
                         "RMDir /r $INSTDIR 会连带用户放入同目录的文件")

    def test_default_apps_registration_removed_on_uninstall(self):
        """「默认应用」登记随卸载清走：RegisteredApplications 值 + Capabilities 树。"""
        self.assertIn(
            'DeleteRegValue HKCU "Software\\RegisteredApplications" "${APPNAME}"',
            self.un, "列表登记值必须单独删（独立键上的值）")
        self.assertIn('DeleteRegKey HKCU "Software\\${APPID}"',
                      self.un, "Capabilities 在 ${APPID} 键下，随整树清理")


class RenderedScriptTests(unittest.TestCase):
    """② 渲染产物断言：真实 builder 走查后，核心钉子在 installer.nsi 上复验。"""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="nsi-polite-")
        cls.tree = Path(cls.tmp.name) / "checkout"
        for rel in ("packaging/nsis/build-installer.py",
                    "packaging/nsis/installer.nsi.tpl",
                    "packaging/version.py"):
            dst = cls.tree / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / rel, dst)
        (cls.tree / "Cargo.toml").write_text(
            "[workspace.package]\nversion = \"0.1.0\"\n", encoding="utf-8")
        exe = cls.tree / "mang-shell/target/release/mang-shell.exe"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b"stub exe")
        dll = cls.tree / "bin/lib/pdfium.dll"
        dll.parent.mkdir(parents=True)
        dll.write_bytes(b"stub dll")
        proc = subprocess.run(
            [sys.executable, str(cls.tree / "packaging/nsis/build-installer.py"),
             "--skip-makensis"],
            capture_output=True, text=True, encoding="utf-8", timeout=120)
        assert proc.returncode == 0, f"builder 失败：{proc.stdout}{proc.stderr}"
        cls.nsi = (cls.tree / "build/nsis/installer.nsi").read_text(encoding="utf-8-sig")

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_rendered_preserves_per_user_surface(self):
        nsi = strip_comments(self.nsi)
        self.assertNotIn("HKLM", nsi)
        self.assertIn("RequestExecutionLevel user", nsi)
        self.assertIn('InstallDir "$LOCALAPPDATA\\Programs\\${APPEXEC}"', nsi)

    def test_rendered_preserves_polite_and_default_apps_guards(self):
        nsi = strip_comments(self.nsi)
        self.assertIn('ReadRegStr $Tmp0 HKCR ".md" ""', nsi)
        self.assertIn('${OrIf} $Tmp0 == "${PROGID}"', nsi)
        self.assertIn('${If} $TypeFailKey != ""', nsi)
        self.assertIn('Software\\RegisteredApplications', nsi)
        self.assertIn('Capabilities\\FileAssociations', nsi)

    def test_rendered_placeholder_zero_residue(self):
        self.assertEqual(self.nsi.count("{{"), 0, "渲染产物不得残留占位符")

    def test_makensis_compiles_rendered_script(self):
        """③ 编译面：makensis 可得时编译；不可得如实 SKIP。"""
        makensis = shutil.which("makensis")
        if makensis is None:
            for cand in (r"C:\Program Files (x86)\NSIS\makensis.exe",
                         r"C:\Program Files\NSIS\makensis.exe"):
                if Path(cand).is_file():
                    makensis = cand
                    break
        if makensis is None:
            self.skipTest("makensis 不可得（安装：Debian `apt-get install nsis` / Windows `choco install nsis`）")
        proc = subprocess.run([makensis, str(self.tree / "build/nsis/installer.nsi")],
                              capture_output=True, text=True, encoding="utf-8", timeout=300)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
