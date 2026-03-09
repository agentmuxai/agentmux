// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// File operations for drag & drop support.

/// Normalize a path that may be in POSIX or mixed-slash form into a native OS path.
/// On Windows handles:
///   "/c/foo/bar"  → "C:\foo\bar"   (MSYS2/Git Bash POSIX drive mounts)
///   "C:/foo/bar"  → "C:\foo\bar"   (forward-slash Windows paths from OSC 7)
/// On other platforms: no-op.
fn normalize_path_for_platform(path: &str) -> String {
    #[cfg(windows)]
    {
        // MSYS2 style: /c/... or /C/...
        if let Some(rest) = path.strip_prefix('/') {
            let mut chars = rest.chars();
            if let Some(drive) = chars.next() {
                if drive.is_ascii_alphabetic() {
                    let after_drive = chars.as_str();
                    if after_drive.is_empty() || after_drive.starts_with('/') {
                        let tail = after_drive.replace('/', "\\");
                        return format!("{}:{}", drive.to_ascii_uppercase(), tail);
                    }
                }
            }
        }
        // Forward-slash Windows path: C:/...
        path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    path.to_string()
}

fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    if src.is_file() {
        std::fs::copy(src, dst).map_err(|e| format!("Copy failed: {}", e))?;
    } else if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("Create dir failed: {}", e))?;
        for entry in std::fs::read_dir(src).map_err(|e| format!("Read dir failed: {}", e))? {
            let entry = entry.map_err(|e| format!("Dir entry error: {}", e))?;
            let name = entry.file_name();
            copy_recursive(&entry.path(), &dst.join(&name))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_file_to_dir(
    source_path: String,
    target_dir: String,
) -> Result<String, String> {
    let source = std::path::Path::new(&source_path);
    // Normalize the target path, handling two Windows-on-POSIX formats:
    //   C:/Users/foo   → C:\Users\foo  (OSC 7 from pwsh, forward-slash Windows path)
    //   /c/Users/foo   → C:\Users\foo  (OSC 7 from bash/MSYS2, POSIX-style drive mount)
    let target_dir_norm = normalize_path_for_platform(&target_dir);
    let target_dir = std::path::Path::new(&target_dir_norm);

    if !source.exists() {
        return Err(format!("Source not found: {}", source.display()));
    }

    if !target_dir.exists() {
        return Err(format!("Target directory not found: {}", target_dir.display()));
    }

    if !target_dir.is_dir() {
        return Err(format!("Target path is not a directory: {}", target_dir.display()));
    }

    let name = source
        .file_name()
        .ok_or_else(|| "Invalid source path".to_string())?;
    let target = target_dir.join(name);

    if target.exists() {
        return Err(format!("Already exists: {}", target.display()));
    }

    copy_recursive(source, &target)?;

    Ok(target.display().to_string())
}
