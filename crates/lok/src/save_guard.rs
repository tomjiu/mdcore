//! Source-file protection shared by the public client and the isolated worker.
//! URL decoding uses `url`; file identity uses OS handles through `same-file`.

use std::path::PathBuf;

fn local_file(url: &url::Url) -> Result<PathBuf, String> {
    if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some() {
        return Err("saveAs requires an unambiguous local file URL".into());
    }
    url.to_file_path()
        .map_err(|_| "invalid local file URL".into())
}

pub(crate) fn check(source: &str, target: &str) -> Result<(), String> {
    let target_url = url::Url::parse(target).map_err(|_| "invalid saveAs URL")?;
    let target_path = local_file(&target_url)?;
    let source_url = url::Url::parse(source).map_err(|_| "invalid source URL")?;
    // A new private:factory document has no source file to protect.
    if source_url.scheme() != "file" {
        return Ok(());
    }
    let source_path = local_file(&source_url)?;
    if source_path == target_path {
        return Err("saveAs must write a new file, not overwrite the source".into());
    }
    // 守卫是「确认性」防护：只在能**肯定**目标已存在时才做同源别名检查。
    // 探不出存在性（父路径为普通文件的 ENOTDIR、不可访问等）一律放行——
    // 这种目标不可能是已存在的源文件，且内核 saveAs 会给出真实的拒绝
    // 原因（E_SAVE），比守卫在宿主侧拦成 E_PROTOCOL 更诚实（smoke 步骤
    // save.denied_keeps_dirty 的既有口径：拒绝语义由内核报，脏位保留）。
    match target_path.try_exists() {
        Ok(true) => {}
        Ok(false) | Err(_) => return Ok(()),
    }
    match same_file::is_same_file(&source_path, &target_path) {
        Ok(true) => Err("saveAs target aliases the source file".into()),
        Ok(false) => Ok(()),
        Err(e) => Err(format!("cannot verify save target identity: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::check;

    /// 拒写语义回归：目标是「父路径为普通文件」的非法路径时必须放行
    /// （内核 saveAs 才是拒绝的权威裁决者，报 E_SAVE；守卫拦成
    /// E_PROTOCOL 会让 smoke save.denied_keeps_dirty 红——2026-11-11
    /// main 实测存量红，本批修复）。
    #[test]
    fn target_under_file_parent_passes_through_to_kernel() {
        let dir = std::env::temp_dir().join(format!("mdc-lok-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker.txt");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let src = dir.join("src.docx");
        std::fs::write(&src, b"src").unwrap();
        let src_url = format!("file://{}", src.to_string_lossy());
        let target_url = format!("file://{}", blocker.join("child.docx").to_string_lossy());
        assert!(
            check(&src_url, &target_url).is_ok(),
            "父为文件的目标：守卫放行（存在性探不出 ≠ 已存在；内核报 E_SAVE）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_equal_to_source_rejected() {
        let dir = std::env::temp_dir().join(format!("mdc-lok-guard-eq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.docx");
        std::fs::write(&src, b"src").unwrap();
        let src_url = format!("file://{}", src.to_string_lossy());
        assert!(
            check(&src_url, &src_url).is_err(),
            "覆盖源文件必须拒（保存守卫的核心职责）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_existing_target_and_factory_source_pass() {
        let dir = std::env::temp_dir().join(format!("mdc-lok-guard-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.docx");
        std::fs::write(&src, b"src").unwrap();
        let dst = dir.join("dst.docx");
        std::fs::write(&dst, b"dst").unwrap();
        let src_url = format!("file://{}", src.to_string_lossy());
        let dst_url = format!("file://{}", dst.to_string_lossy());
        assert!(check(&src_url, &dst_url).is_ok(), "存在的不同目标放行");
        // private:factory 新文档无源可保护
        assert!(check("private:factory/swriter", &dst_url).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
