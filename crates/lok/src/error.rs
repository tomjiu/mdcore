//! 错误分类（#633）。
//!
//! 纪律：宿主消费 worker 响应必须拿到「明确错误」——每个失败面
//! 有独立 code 与可行动的信息（缺失运行时 / 运行时过旧缺成员 /
//! worker 死亡 / 请求超时 / 帧违规 / 保存失败 / 校验失败），禁止
//! 吞错或把失败伪装成空成功（AGENTS「不静默失败」纪律）。

use std::fmt;
use std::io;

/// 错误码（稳定字符串，进帧协议与日志，便于 #616 宿主区分处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// 宿主侧：找不到 LibreOffice 运行时（库文件/安装目录不存在）。
    RuntimeNotFound,
    /// dlopen/LoadLibrary 失败或 hook 符号缺失（发行版构建问题）。
    LibraryLoadFailed,
    /// 运行时 vtable 缺成员（nSize 检查不过）——版本过旧。
    RuntimeTooOld,
    /// LibreOffice 初始化（hook）失败。
    InitFailed,
    /// 文档加载失败（getError 文本随附）。
    LoadFailed,
    /// 文档尚未加载 / 已关闭（单文档 worker 状态机）。
    NoDocument,
    /// 已有文档在编辑（原型单文档，需先 close）。
    DocumentAlreadyOpen,
    /// worker 进程死亡（EOF / 非零退出 / 被杀）。
    WorkerDied,
    /// 请求超时（宿主侧截止时间到，worker 可能仍在处理）。
    RequestTimeout,
    /// 帧协议违规（坏 magic / 超长 / 截断 / id 不匹配）。
    BadFrame,
    /// 保存失败（目标拒写 / saveAs 返回失败 / 校验不过）。
    SaveFailed,
    /// 保存产物校验失败（zip 结构 / 期望文本缺失）。
    VerifyFailed,
    /// 宿主侧参数/状态错误（如 save 目标 == 源文件路径）。
    InvalidRequest,
    /// 其他 IO。
    Io,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::RuntimeNotFound => "runtime-not-found",
            ErrorCode::LibraryLoadFailed => "library-load-failed",
            ErrorCode::RuntimeTooOld => "runtime-too-old",
            ErrorCode::InitFailed => "init-failed",
            ErrorCode::LoadFailed => "load-failed",
            ErrorCode::NoDocument => "no-document",
            ErrorCode::DocumentAlreadyOpen => "document-already-open",
            ErrorCode::WorkerDied => "worker-died",
            ErrorCode::RequestTimeout => "request-timeout",
            ErrorCode::BadFrame => "bad-frame",
            ErrorCode::SaveFailed => "save-failed",
            ErrorCode::VerifyFailed => "verify-failed",
            ErrorCode::InvalidRequest => "invalid-request",
            ErrorCode::Io => "io",
        }
    }

    /// 帧协议错误码 → 枚举（未知码归 Io，原文信息在 message 里）。
    pub fn from_code(s: &str) -> Self {
        match s {
            "runtime-not-found" => ErrorCode::RuntimeNotFound,
            "library-load-failed" => ErrorCode::LibraryLoadFailed,
            "runtime-too-old" => ErrorCode::RuntimeTooOld,
            "init-failed" => ErrorCode::InitFailed,
            "load-failed" => ErrorCode::LoadFailed,
            "no-document" => ErrorCode::NoDocument,
            "document-already-open" => ErrorCode::DocumentAlreadyOpen,
            "worker-died" => ErrorCode::WorkerDied,
            "request-timeout" => ErrorCode::RequestTimeout,
            "bad-frame" => ErrorCode::BadFrame,
            "save-failed" => ErrorCode::SaveFailed,
            "verify-failed" => ErrorCode::VerifyFailed,
            "invalid-request" => ErrorCode::InvalidRequest,
            _ => ErrorCode::Io,
        }
    }
}

/// LOK 面（宿主库 + worker 共用）的统一错误。
#[derive(Debug, Clone)]
pub struct LokError {
    pub code: ErrorCode,
    pub message: String,
}

impl LokError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        LokError {
            code,
            message: message.into(),
        }
    }

    pub fn runtime_not_found(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::RuntimeNotFound, detail)
    }

    pub fn too_old(member: &str, n_size: usize, need: usize) -> Self {
        Self::new(
            ErrorCode::RuntimeTooOld,
            format!(
                "LibreOffice 运行时过旧：vtable nSize={n_size}，缺成员 {member} \
                 （需 offset<{need}）。请换用 ≥6.0 的完整安装；运行时发现方式\
                 见 docs/lok-runtime.md"
            ),
        )
    }
}

impl fmt::Display for LokError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for LokError {}

impl From<io::Error> for LokError {
    fn from(e: io::Error) -> Self {
        LokError::new(ErrorCode::Io, format!("IO 错误：{e}"))
    }
}

pub type LokResult<T> = Result<T, LokError>;
