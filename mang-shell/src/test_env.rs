//! 测试专用的进程级环境互斥（`#[cfg(test)]` 编译面）。
//!
//! 根因：`XDG_CONFIG_HOME` 是**进程级全局**——并行测试族里多个测试各自
//! `set_var` 指向自己的隔离目录再 restore，期间任何**别的测试**读
//! `config_dir()` 都会读到别人目录（例：`settings::user_theme_replace_saved_
//! roundtrip` 的 `load_user_themes` 计数在 `document::fold_persistence_
//! roundtrip_on_reopen` 持 XDG 窗口内插队 → 读到别人 themes 目录 → 计数错
//! → 间歇红（2/3 复现率实测）。
//!
//! 修法：持全局 XDG 的测试全部包 [`xdg_lock`]——同进程内文件锁互斥
//! （Linux `flock` 对同进程多 fd 仍互斥）。锁文件 `/tmp/mang-xdg-test.lock`
//! 内容无意义，只做互斥点。

#![cfg(test)]

use std::fs::File;
use std::os::unix::io::AsRawFd;

/// 拿进程内 XDG 互斥锁（拿不到 = 等待持有者出窗）。测试用——
/// 持锁期间可安全 `set_var("XDG_CONFIG_HOME")` + 读 `config_dir()` 链。
pub(crate) fn xdg_lock() -> File {
    let f = File::create("/tmp/mang-xdg-test.lock").expect("建 XDG 测试锁文件");
    // stable 面 flock 系统调用（LOCK_EX = 互斥; 同进程多 fd 仍互斥——
    // 正是要的语义）。阻塞等持有者出窗。
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    assert_eq!(rc, 0, "flock LOCK_EX");
    f
}
