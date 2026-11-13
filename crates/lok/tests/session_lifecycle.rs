//! B3 运行时与多会话生命周期测试（#49 工作线 B）。
//!
//! 验证：反复打开关闭（profile 锁不残留 / worker 死透）、双会话并发
//! 编辑保存互不干扰、启动失败可操作错误、worker 外杀死亡扩散与恢复、
//! runtime 来源/hash 登记。环境门：无 LibreOffice 运行时 SKIP（≠PASS）。

use mdc_lok::client::DocSession;
use std::path::PathBuf;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-b3-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap();
    mdc_lok::profile::file_url_of(&canon)
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // 0 信号探测（unix）；kill(0) = 权限探测不实际发信号
        unsafe { libc_kill_0(pid as i32) == 0 }
    }
    #[cfg(windows)]
    {
        // 【2026-11-12 补齐 Windows 实测门】原实现恒返回 true（桩），使
        // 「shutdown 后进程死透」无从验证。改用 OpenProcess +
        // WaitForSingleObject(0)：进程已终止（即便句柄尚未 wait 回收）→
        // WAIT_OBJECT_0；仍在运行 → WAIT_TIMEOUT。
        type Handle = *mut std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
            fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
            fn CloseHandle(h: Handle) -> i32;
        }
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const WAIT_TIMEOUT: u32 = 0x0000_0102;
        unsafe {
            let h = OpenProcess(SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false; // 打不开 = 进程已不存在
            }
            let r = WaitForSingleObject(h, 0);
            CloseHandle(h);
            r == WAIT_TIMEOUT
        }
    }
}

#[cfg(unix)]
unsafe fn libc_kill_0(pid: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, 0)
}

/// 【2026-11-12 修复并行污染】改「进程级环境变量」的测试（如垃圾 runtime
/// 用例会 remove_var(MDC_LOK_HOME) + set_var(MDC_LOK_LIB=<junk>)）在同进程
/// 多线程并行下会**污染同时运行的其它测试**：实测并行跑时
/// `external_kill_death_and_recovery` / `two_sessions_concurrent_edit_and_save`
/// 的 DocSession::open 读到垃圾 env，直接 E_LOK_LOAD GetLastError=193 失败，
/// 而单跑必过——典型测试隔离缺陷（非产品缺陷）。此处用全局锁把这类用例
/// 串行化；其余用例正常并行。
static ENV_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// 读锁：正常用例（并行；互不干扰地读环境变量）。
fn env_read() -> std::sync::RwLockReadGuard<'static, ()> {
    ENV_LOCK.read().unwrap_or_else(|e| e.into_inner())
}

/// 写锁：独占修改进程级环境变量的用例（与所有读锁用例互斥）。
fn env_write() -> std::sync::RwLockWriteGuard<'static, ()> {
    ENV_LOCK.write().unwrap_or_else(|e| e.into_inner())
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

/// 反复打开关闭 ×5：每次都能起新 worker（profile 锁不残留），shutdown
/// 后 worker 进程死透（不泄漏）。
#[test]
fn repeated_open_shutdown_cycles() {
    let _env_guard = env_read();
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("cycles");
    let work = dir.join("w.docx");
    std::fs::copy(fixture(), &work).unwrap();

    let mut pids = std::collections::HashSet::new();
    for i in 0..5 {
        let s = DocSession::open(None, &dir.join(format!("c{i}")), &file_url(&work), None)
            .unwrap_or_else(|e| panic!("第 {i} 轮 open 失败（profile 锁残留？）：{e}"));
        let (w, _h) = s.size().expect("size");
        assert!(w > 0, "第 {i} 轮几何可用");
        pids.insert(s.pid());
        s.shutdown();
    }
    // shutdown 后全部 worker 死透（等待上限 10s；SIGKILL 后立即，留余量）
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let alive: Vec<_> = pids.iter().filter(|p| pid_alive(**p)).copied().collect();
        if alive.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let alive: Vec<_> = pids.iter().filter(|p| pid_alive(**p)).copied().collect();
    assert!(
        alive.is_empty(),
        "shutdown 后 worker 进程死透（仍存活：{alive:?}——孤儿泄漏违反隔离契约）"
    );
    assert_eq!(pids.len(), 5, "每轮独立 worker 进程（pid 互异）");
}

/// 双会话并发编辑 + 保存：互不干扰，产物各自正确。
#[test]
fn two_sessions_concurrent_edit_and_save() {
    let _env_guard = env_read();
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("dual");
    let w1 = dir.join("a.docx");
    let w2 = dir.join("b.docx");
    std::fs::copy(fixture(), &w1).unwrap();
    std::fs::copy(fixture(), &w2).unwrap();
    let saved_url = |name: &str| -> String {
        let canon = std::fs::canonicalize(&dir).unwrap();
        mdc_lok::profile::file_url_of(&canon.join(name))
    };

    let mut s1 = DocSession::open(None, &dir.join("s1"), &file_url(&w1), None).expect("会话 1");
    let mut s2 = DocSession::open(None, &dir.join("s2"), &file_url(&w2), None)
        .expect("会话 2（并行，独立 profile）");
    assert_ne!(s1.pid(), s2.pid(), "两会话独立 worker 进程");

    // 全选替换（会话 1 文本甲 / 会话 2 文本乙）
    s1.uno(".uno:SelectAll", None).expect("s1 全选");
    s1.type_text("会话一内容").expect("s1 编辑");
    s2.uno(".uno:SelectAll", None).expect("s2 全选");
    s2.type_text("会话二内容").expect("s2 编辑");

    // 保存与重开校验（交叉断言：甲产物不含乙文本）
    s1.save_as(&saved_url("a-saved.docx"), "docx")
        .expect("s1 保存");
    s2.save_as(&saved_url("b-saved.docx"), "docx")
        .expect("s2 保存");
    assert!(dir.join("a-saved.docx").is_file());
    assert!(dir.join("b-saved.docx").is_file());

    let mut v1 =
        DocSession::open(None, &dir.join("v1"), &saved_url("a-saved.docx"), None).expect("重开 a");
    let mut v2 =
        DocSession::open(None, &dir.join("v2"), &saved_url("b-saved.docx"), None).expect("重开 b");
    v1.uno(".uno:SelectAll", None).expect("v1 全选");
    for _ in 0..3 {
        let (_, t1) = v1
            .text_selection("text/plain;charset=utf-8")
            .expect("v1 选区");
        if t1.contains("会话一内容") && !t1.contains("会话二内容") {
            break;
        }
        v1.uno(".uno:SelectAll", None).expect("v1 全选重试");
    }
    let (_, t1) = v1.text_selection("text/plain;charset=utf-8").unwrap();
    assert!(
        t1.contains("会话一内容") && !t1.contains("会话二内容"),
        "会话 1 产物只含自己的编辑（隔离；实际：{t1:?}）"
    );
    let (_, t2) = {
        v2.uno(".uno:SelectAll", None).expect("v2 全选");
        v2.text_selection("text/plain;charset=utf-8")
            .expect("v2 选区")
    };
    assert!(
        t2.contains("会话二内容") && !t2.contains("会话一内容"),
        "会话 2 产物只含自己的编辑（隔离；实际：{t2:?}）"
    );

    s1.shutdown();
    s2.shutdown();
    v1.shutdown();
    v2.shutdown();
}

/// 启动失败可操作错误：MDC_LOK_LIB 指向垃圾文件 → 明确失败（路径 +
/// 补救提示），不挂死不 panic。
#[test]
fn bad_runtime_env_actionable_error() {
    // 独占：本用例修改进程级 env（remove MDC_LOK_HOME / 设垃圾 MDC_LOK_LIB），
    // 与所有读锁用例互斥，避免并行时污染他人（见 ENV_LOCK 注释）。
    let _env_guard = env_write();
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("badenv");
    let work = dir.join("w.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let junk = dir.join("junk.so");
    std::fs::write(&junk, b"not a shared library").unwrap();

    // 【2026-11-12 补齐】原实现只覆盖 MDC_LOK_LIB，但发现序 `discover_paths`
    // **先看 MDC_LOK_HOME**：若测试宿主导出了有效的 HOME（CI/本地启动脚本
    // 常这么做），第 1 步就命中真运行时，垃圾 LIB 根本没被评估 → open 成功，
    // 断言必然失败。因此这里同时接管两个变量，构造「显式配置的运行时无效」
    // 这一真实用户场景。
    let prev_home = std::env::var_os("MDC_LOK_HOME");
    let prev_lib = std::env::var_os("MDC_LOK_LIB");
    std::env::remove_var("MDC_LOK_HOME");
    std::env::set_var("MDC_LOK_LIB", &junk);
    let r = DocSession::open(None, &dir.join("s"), &file_url(&work), None);
    match prev_home {
        Some(v) => std::env::set_var("MDC_LOK_HOME", v),
        None => std::env::remove_var("MDC_LOK_HOME"),
    }
    match prev_lib {
        Some(v) => std::env::set_var("MDC_LOK_LIB", v),
        None => std::env::remove_var("MDC_LOK_LIB"),
    }
    let err = r.expect_err("垃圾 runtime 必须失败");
    let msg = format!("{err}");
    assert!(
        msg.contains("dlopen") || msg.contains("发现失败") || msg.contains("hook"),
        "错误可操作（指明 dlopen/hook/发现失败；实际：{msg}）"
    );
    eprintln!("垃圾 runtime 错误信息（可操作性样本）：{msg}");
}

/// worker 外杀 → 死亡扩散（is_dead + 后续请求 WorkerDied）→ 宿主开新
/// 会话恢复；源文件字节不变。
#[test]
fn external_kill_death_and_recovery() {
    let _env_guard = env_read();
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("kill");
    let work = dir.join("w.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let src_hash_before = sha256_of(&work);

    let s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("会话");
    // 外杀路径两平台共用（Windows 走 taskkill /F）
    let pid = s.pid();

    // 外杀 worker（模拟崩溃）
    #[cfg(unix)]
    {
        let out = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status();
        assert!(out.is_ok(), "kill -9 发出");
    }
    // 【2026-11-12 补齐 Windows 实测门】原实现是 unix 专属：Windows 上
    // 根本没人杀进程，is_dead 永远 false，本契约必然失败。改用
    // taskkill /F /PID（TerminateProcess 语义）；之后读线程 stdout EOF
    // → mark_dead 完成死亡扩散，正是本用例要验的路径。
    #[cfg(windows)]
    {
        let out = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status();
        assert!(out.is_ok(), "taskkill /F 发出");
    }
    // 等死亡扩散（读线程 EOF → dead 标志）
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !s.is_dead() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(s.is_dead(), "外杀后 is_dead=true（死亡扩散）");
    let err = s.size().expect_err("死后请求必须明确失败");
    assert!(
        matches!(err, mdc_lok::client::LokError::WorkerDied { .. }),
        "死后错误类型 WorkerDied（实际：{err}）"
    );

    // 宿主恢复：新会话
    let s2 =
        DocSession::open(None, &dir.join("s2"), &file_url(&work), None).expect("外杀后新会话恢复");
    let (w, _h) = s2.size().expect("恢复会话可用");
    assert!(w > 0);
    s2.shutdown();

    // 源文件不受影响
    assert_eq!(
        sha256_of(&work),
        src_hash_before,
        "worker 被杀源文件字节不变（隔离铁律）"
    );
}

/// runtime 来源/hash 登记：discover() 提供内核库路径 + SHA-256（版本
/// 钉定验收项）。
#[test]
fn runtime_source_and_hash_registered() {
    let _env_guard = env_read();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let rt = mdc_lok::runtime::discover().expect("发现运行时");
    assert!(rt.program_dir.is_dir(), "program 目录存在");
    assert!(rt.kernel_lib.is_file(), "内核库文件存在");
    assert_eq!(
        rt.kernel_sha256.len(),
        64,
        "SHA-256 十六进制 64 字符（实际 {}）",
        rt.kernel_sha256.len()
    );
    // hash 与实际文件一致（独立重算）
    assert_eq!(
        rt.kernel_sha256,
        sha256_of(&rt.kernel_lib),
        "登记 hash 与文件实际内容一致（来源可信）"
    );
}

fn sha256_of(p: &std::path::Path) -> String {
    let data = std::fs::read(p).unwrap();
    mdc_lok::sha256::sha256_hex(&data)
}
