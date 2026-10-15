// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `SendMessage`'s `files`: each file is put in this agent's own attachment
//! store first, through srv's upload route, and the jekt then names them by
//! hash (docs/specs/SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §3.1). This
//! process runs as the agent, so it sends exactly what the agent can read.

use agentmux_common::jekt_files::{self, JektFile};

use super::*;

/// The most one file, and all of a jekt's files together, may weigh on the
/// same machine (spec §5).
pub(super) const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;
/// An upload waits for srv to store and process the file (thumbnails, a
/// document's text version), well past the client's usual 10 s.
const UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The `files` argument: `None` when absent or empty.
pub(super) fn files_arg(arguments: &Value) -> Result<Option<Vec<String>>> {
    let Some(v) = arguments.get("files").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let list = v.as_array().ok_or_else(|| anyhow::anyhow!("files must be a list of file paths"))?;
    let paths = list
        .iter()
        .map(|p| {
            p.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("files must be a list of file paths"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((!paths.is_empty()).then_some(paths))
}

/// One file to send: where it is and how big it is.
#[derive(Debug)]
pub(super) struct Local {
    pub path: std::path::PathBuf,
    pub name: String,
    pub size: u64,
}

/// Check every path before anything is uploaded: each names a readable,
/// non-empty file within the limits. The error names the file.
pub(super) fn check(paths: &[String]) -> Result<Vec<Local>> {
    if paths.len() > jekt_files::MAX_FILES {
        anyhow::bail!("{} files is more than the limit of {} per message", paths.len(), jekt_files::MAX_FILES);
    }
    let mut out = Vec::with_capacity(paths.len());
    let mut total = 0u64;
    for given in paths {
        let path = std::path::PathBuf::from(resolve_workdir_path(given));
        let meta = std::fs::metadata(&path).map_err(|e| anyhow::anyhow!("{given}: {e}"))?;
        if meta.is_dir() {
            anyhow::bail!("{given} is a folder; send the files in it, or an archive of it");
        }
        let size = meta.len();
        if size == 0 {
            anyhow::bail!("{given} is empty");
        }
        if size > MAX_FILE_BYTES {
            anyhow::bail!("{given} is {} MB, over the {} MB limit per file", size / (1024 * 1024), MAX_FILE_BYTES / (1024 * 1024));
        }
        total += size;
        if total > MAX_TOTAL_BYTES {
            anyhow::bail!("these files come to more than the {} MB limit per message", MAX_TOTAL_BYTES / (1024 * 1024));
        }
        // Readable now, not only listed.
        std::fs::File::open(&path).map_err(|e| anyhow::anyhow!("{given}: {e}"))?;
        let name = jekt_files::clean_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        out.push(Local { path, name, size });
    }
    Ok(out)
}

/// Upload each file into this agent's attachment store and return the
/// manifest, in the order given.
pub(super) async fn upload(files: Vec<Local>, cx: &ToolCtx<'_>) -> Result<Vec<JektFile>> {
    let url = format!("{}/api/v1/attachments/upload", cx.local_url.trim_end_matches('/'));
    let mut manifest = Vec::with_capacity(files.len());
    for f in files {
        let file = tokio::fs::File::open(&f.path)
            .await
            .map_err(|e| anyhow::anyhow!("{}: {e}", f.path.display()))?;
        let resp = cx
            .client
            .post(&url)
            .query(&[("name", f.name.as_str())])
            .header(AUTH_KEY_HEADER, cx.auth_key)
            .timeout(UPLOAD_TIMEOUT)
            .body(reqwest::Body::from(file))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}: upload failed: {e}", f.name))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or_default();
            let why = body.get("error").and_then(Value::as_str).unwrap_or("");
            anyhow::bail!("{}: upload failed: HTTP {status} {why}", f.name);
        }
        let info: Value = resp.json().await.map_err(|e| anyhow::anyhow!("{}: upload answer unreadable: {e}", f.name))?;
        let sha256 = info.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        // What srv stored, not what was on disk a moment ago.
        let size = info.get("bytes").and_then(Value::as_u64).unwrap_or(f.size);
        manifest.push(JektFile { sha256, name: f.name, size });
    }
    jekt_files::validate(&manifest).map_err(|e| anyhow::anyhow!("files refused: {e}"))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_arg_reads_a_list_and_ignores_an_empty_one() {
        assert_eq!(files_arg(&serde_json::json!({})).unwrap(), None);
        assert_eq!(files_arg(&serde_json::json!({ "files": [] })).unwrap(), None);
        assert_eq!(files_arg(&serde_json::json!({ "files": null })).unwrap(), None);
        assert_eq!(
            files_arg(&serde_json::json!({ "files": [" a.png ", "b.txt"] })).unwrap(),
            Some(vec!["a.png".to_string(), "b.txt".to_string()])
        );
        assert!(files_arg(&serde_json::json!({ "files": "a.png" })).is_err());
        assert!(files_arg(&serde_json::json!({ "files": [""] })).is_err());
        assert!(files_arg(&serde_json::json!({ "files": [1] })).is_err());
    }

    #[test]
    fn check_names_the_file_it_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("shot.png");
        std::fs::write(&ok, b"png").unwrap();
        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        let p = |p: &std::path::Path| p.to_string_lossy().into_owned();

        let got = check(&[p(&ok)]).unwrap();
        assert_eq!((got[0].name.as_str(), got[0].size), ("shot.png", 3));

        let err = check(&[p(&empty)]).unwrap_err().to_string();
        assert!(err.contains("empty.txt") && err.contains("empty"), "{err}");
        let err = check(&[p(dir.path())]).unwrap_err().to_string();
        assert!(err.contains("is a folder"), "{err}");
        let missing = p(&dir.path().join("nope.png"));
        assert!(check(&[missing.clone()]).unwrap_err().to_string().contains(&missing));
        let many = vec![p(&ok); jekt_files::MAX_FILES + 1];
        assert!(check(&many).unwrap_err().to_string().contains("limit"));
    }
}
