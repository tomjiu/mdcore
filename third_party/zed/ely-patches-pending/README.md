# Ely GPUI 补丁待应用存档

本目录存放**尚未应用**的 ElyOffice GPUI 补丁，供未来 macOS 版本使用。

## 0007-macos-accesskit-answers-from-the-gpui-view.patch

- 来源：ELY-Open-Source/ElyOffice patches/gpui/0007
- 原因：macOS 专用，重构 AccessKit 适配器（~250 行）。
  当前只编 Windows（gpui_macos 在 Windows 上不参与编译），
  打上去无法验证，故存档待 macOS 版本时应用。
- 前置：需要 macOS 环境验证编译通过。

## 已完成（Windows 相关，已应用到 zed 闭包）

| 补丁 | 状态 |
|---|---|
| 0001 坏帧中止图片加载 | ✅ git apply |
| 0002 最上层控件赢得命中测试 | ✅ 手工适配 |
| 0003 测试平台计数退出请求 | ✅ git apply |
| 0004 a11y 调试记录 UTC | ✅ git apply |
| 0005 辅助边界跟随窗口 | ✅ 手工适配 |
| 0006 Linux XIM 上下文 | ✅ git apply |
| 0008 Windows 不对已销毁窗口调用 | ✅ git apply |
| 0009 退出后任务静默丢弃 | ✅ git apply |
| 0010 Windows 汉字布局跟随语言 | ✅ 手工适配 |
| 0011 Windows 无进程 AUMID | ⏭ 上游已解决（zed 已无此调用） |
| 0012 Windows Save As 补扩展名 | ✅ git apply |
| 0013 Windows AccessKit 活动态 | ✅ git apply |
