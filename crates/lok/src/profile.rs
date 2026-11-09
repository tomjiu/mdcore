//! 会话独立 user profile（issue「共享平台 profile 不可复用，多会话不得
//! 互相锁住」）。
//!
//! 每个 worker 会话一个一次性 profile 目录（`<tmp>/mdc-lok/<会话>/profile`），
//! 绝不复用 LO 默认共享 profile（默认 profile 的 `.lock` 会互锁；被杀
//! worker 的残留锁还会挂死下一个——ElyOffice `SAL_NOLOCK_PROFILE=1` 实证，
//! 本 crate 一并采纳）。目录由 [`SessionProfile`] RAII 清理（尽力而为）。

use std::path::{Path, PathBuf};

/// 一次性会话 profile。
pub struct SessionProfile {
    dir: PathBuf,
}

impl SessionProfile {
    /// 建独立 profile 目录（父级 `mdc-lok/<会话>` 由调用方命名）。
    pub fn new(session_root: &Path) -> std::io::Result<SessionProfile> {
        let dir = session_root.join("profile");
        std::fs::create_dir_all(&dir)?;
        Ok(SessionProfile { dir })
    }

    /// profile 目录路径。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `file:` URI（hook_2 第二参契约：必须是 URL 不是路径名；Windows
    /// `\\?\` / `\\?\UNC\` 前缀按 ElyOffice `ipc.rs` 规则转写，其余盘符
    /// `file:///C:/...`；反斜杠统一正斜杠）。
    pub fn file_url(&self) -> String {
        file_url_of(&self.dir)
    }
}

impl Drop for SessionProfile {
    fn drop(&mut self) {
        // 尽力清理；失败不掩盖主流程（残留 profile 无锁，不影响下个会话）
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 路径 → `file:` URI（逐字节 percent-encode，仅保留非保留字符；规则
/// 对齐 ElyOffice `ipc.rs::file_url_of`，可移植面）。
pub fn file_url_of(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");

    #[cfg(target_os = "windows")]
    {
        // \\?\UNC\server\share → file://server/share（前缀 8 字节）
        let low = s.to_lowercase();
        if low.starts_with(r"//?/unc/") {
            return percent_encode_tail(&format!("file://{}", &s[8..]), 7);
        }
        // \\?\C:\... → file:///C:/...（前缀 4 字节）
        if low.starts_with(r"//?/") {
            return percent_encode_tail(&format!("file:///{}", &s[4..]), 8);
        }
    }
    // POSIX 绝对路径：file:// + /abs/... = file:///abs/...（三斜杠）
    percent_encode_tail(&format!("file://{s}"), 7)
}

/// 从 `already` 字节后开始 percent-encode 剩余部分（已写前缀保持原样）。
fn percent_encode_tail(url: &str, already: usize) -> String {
    let bytes = url.as_bytes();
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..already]);
    let keep =
        |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':');
    for &b in &bytes[already..] {
        if keep(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_url_shape() {
        let p = PathBuf::from("/tmp/mdc-lok/s1/profile");
        assert_eq!(file_url_of(&p), "file:///tmp/mdc-lok/s1/profile");
    }

    #[test]
    fn percent_encoding_minimal() {
        let p = PathBuf::from("/tmp/mdc lok/a b/profile"); // 空格 → %20
        assert_eq!(file_url_of(&p), "file:///tmp/mdc%20lok/a%20b/profile");
    }

    #[test]
    fn windows_shapes() {
        // cfg(windows) 分支在 Linux 编译不到，规则以单测形式固定（Windows
        // 实测门未执行——如实注明）；手动驱动内部规则：
        assert_eq!(percent_encode_tail("file:///a b", 8), "file:///a%20b");
    }

    #[test]
    fn chinese_path_roundtrip_shape() {
        // 中文路径必须走 percent-encode（UTF-8 字节逐个转义）
        let p = PathBuf::from("/tmp/会话一/profile");
        let url = file_url_of(&p);
        assert!(url.starts_with("file:///tmp/"));
        assert!(!url.contains("会"), "中文字节必须转义：{url}");
        assert!(url.contains('%'));
    }
}
