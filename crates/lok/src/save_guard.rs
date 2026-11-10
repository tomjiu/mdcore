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
    if !target_path
        .try_exists()
        .map_err(|e| format!("save target lookup failed: {e}"))?
    {
        return Ok(());
    }
    match same_file::is_same_file(&source_path, &target_path) {
        Ok(true) => Err("saveAs target aliases the source file".into()),
        Ok(false) => Ok(()),
        Err(e) => Err(format!("cannot verify save target identity: {e}")),
    }
}
