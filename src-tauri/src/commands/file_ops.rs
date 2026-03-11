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

/// Return a destination path that does not yet exist.
/// "file.txt" → "file_1.txt" → "file_2.txt" … up to _99, then error.
fn deconflict_path(dir: &std::path::Path, name: &std::ffi::OsStr) -> Result<std::path::PathBuf, String> {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }

    let name_str = name.to_string_lossy();
    let (stem, ext) = match name_str.rfind('.') {
        Some(dot) => (&name_str[..dot], &name_str[dot..]),
        None => (name_str.as_ref(), ""),
    };

    for n in 1..=99 {
        let new_name = format!("{stem}_{n}{ext}");
        let candidate = dir.join(&new_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find a free filename for '{}' in '{}'",
        name_str,
        dir.display()
    ))
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

    // Resolve a non-colliding destination path.
    // If "file.txt" already exists, try "file_1.txt", "file_2.txt", … up to 99.
    let target = deconflict_path(target_dir, name)?;

    copy_recursive(source, &target)?;

    Ok(target.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    // ── normalize_path_for_platform ───────────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn test_normalize_msys2_style() {
        assert_eq!(normalize_path_for_platform("/c/Users/foo"), r"C:\Users\foo");
        assert_eq!(normalize_path_for_platform("/C/Users/foo"), r"C:\Users\foo");
        assert_eq!(normalize_path_for_platform("/d/Projects"), r"D:\Projects");
        assert_eq!(normalize_path_for_platform("/z"), "Z:");
    }

    #[cfg(windows)]
    #[test]
    fn test_normalize_forward_slash_windows() {
        assert_eq!(normalize_path_for_platform(r"C:/Users/foo"), r"C:\Users\foo");
        assert_eq!(normalize_path_for_platform(r"C:\Users\foo"), r"C:\Users\foo");
    }

    #[cfg(windows)]
    #[test]
    fn test_normalize_non_drive_path_unchanged_on_windows() {
        // Paths that start with / but are not MSYS2 drive mounts (e.g. /tmp)
        // fall through to the forward-slash replace branch — / stays as \
        assert_eq!(normalize_path_for_platform("/tmp/file.txt"), r"\tmp\file.txt");
    }

    #[cfg(not(windows))]
    #[test]
    fn test_normalize_noop_on_unix() {
        assert_eq!(normalize_path_for_platform("/tmp/file.txt"), "/tmp/file.txt");
        assert_eq!(normalize_path_for_platform("/home/user/foo"), "/home/user/foo");
    }

    // ── copy_recursive ────────────────────────────────────────────────────────

    #[test]
    fn test_copy_recursive_single_file() {
        let dir = tempdir();
        let src_file = dir.join("source.txt");
        let dst_file = dir.join("dest.txt");
        fs::write(&src_file, b"hello world").unwrap();

        copy_recursive(&src_file, &dst_file).unwrap();

        assert!(dst_file.exists());
        assert_eq!(fs::read_to_string(&dst_file).unwrap(), "hello world");
    }

    #[test]
    fn test_copy_recursive_directory() {
        let dir = tempdir();
        let src_dir = dir.join("src_folder");
        let dst_dir = dir.join("dst_folder");
        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("a.txt"), b"aaa").unwrap();
        fs::write(src_dir.join("b.txt"), b"bbb").unwrap();
        let nested = src_dir.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("c.txt"), b"ccc").unwrap();

        copy_recursive(&src_dir, &dst_dir).unwrap();

        assert!(dst_dir.join("a.txt").exists());
        assert!(dst_dir.join("b.txt").exists());
        assert!(dst_dir.join("nested").join("c.txt").exists());
        assert_eq!(fs::read_to_string(dst_dir.join("nested").join("c.txt")).unwrap(), "ccc");
    }

    // ── copy_file_to_dir (sync wrapper for testing) ───────────────────────────

    fn copy_file_to_dir_sync(source_path: &str, target_dir: &str) -> Result<String, String> {
        let source = Path::new(source_path);
        let target_dir_norm = normalize_path_for_platform(target_dir);
        let target_dir_path = Path::new(&target_dir_norm);

        if !source.exists() {
            return Err(format!("Source not found: {}", source.display()));
        }
        if !target_dir_path.exists() {
            return Err(format!("Target directory not found: {}", target_dir_path.display()));
        }
        if !target_dir_path.is_dir() {
            return Err(format!("Target path is not a directory: {}", target_dir_path.display()));
        }
        let name = source.file_name().ok_or_else(|| "Invalid source path".to_string())?;
        let target = deconflict_path(target_dir_path, name)?;
        copy_recursive(source, &target)?;
        Ok(target.display().to_string())
    }

    #[test]
    fn test_copy_file_to_dir_success() {
        let dir = tempdir();
        let src = dir.join("myfile.txt");
        let dst_dir = dir.join("output");
        fs::create_dir(&dst_dir).unwrap();
        fs::write(&src, b"content").unwrap();

        let result = copy_file_to_dir_sync(src.to_str().unwrap(), dst_dir.to_str().unwrap());
        assert!(result.is_ok(), "expected ok, got: {:?}", result);
        assert!(dst_dir.join("myfile.txt").exists());
    }

    #[test]
    fn test_copy_file_to_dir_missing_source() {
        let dir = tempdir();
        let result = copy_file_to_dir_sync(
            dir.join("nonexistent.txt").to_str().unwrap(),
            dir.to_str().unwrap(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Source not found"));
    }

    #[test]
    fn test_copy_file_to_dir_missing_target() {
        let dir = tempdir();
        let src = dir.join("file.txt");
        fs::write(&src, b"data").unwrap();

        let result = copy_file_to_dir_sync(
            src.to_str().unwrap(),
            dir.join("no_such_dir").to_str().unwrap(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Target directory not found"));
    }

    #[test]
    fn test_copy_file_to_dir_target_is_file_not_dir() {
        let dir = tempdir();
        let src = dir.join("src.txt");
        let not_a_dir = dir.join("not_a_dir.txt");
        fs::write(&src, b"data").unwrap();
        fs::write(&not_a_dir, b"other").unwrap();

        let result = copy_file_to_dir_sync(
            src.to_str().unwrap(),
            not_a_dir.to_str().unwrap(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a directory"));
    }

    #[test]
    fn test_copy_file_to_dir_deconflicts_on_collision() {
        let dir = tempdir();
        let src = dir.join("file.txt");
        let dst_dir = dir.join("out");
        fs::create_dir(&dst_dir).unwrap();
        fs::write(&src, b"data").unwrap();
        // Pre-create "file.txt" and "file_1.txt" so deconflict should land on "file_2.txt"
        fs::write(dst_dir.join("file.txt"), b"existing").unwrap();
        fs::write(dst_dir.join("file_1.txt"), b"also existing").unwrap();

        let result = copy_file_to_dir_sync(src.to_str().unwrap(), dst_dir.to_str().unwrap());
        assert!(result.is_ok(), "expected ok, got: {:?}", result);
        let dest = result.unwrap();
        assert!(dest.ends_with("file_2.txt"), "expected file_2.txt, got: {dest}");
        assert!(dst_dir.join("file_2.txt").exists());
    }

    #[test]
    fn test_deconflict_path_no_collision() {
        let dir = tempdir();
        let name = std::ffi::OsStr::new("hello.txt");
        let result = deconflict_path(&dir, name).unwrap();
        assert_eq!(result, dir.join("hello.txt"));
    }

    #[test]
    fn test_deconflict_path_with_collision() {
        let dir = tempdir();
        fs::write(dir.join("hello.txt"), b"").unwrap();
        let name = std::ffi::OsStr::new("hello.txt");
        let result = deconflict_path(&dir, name).unwrap();
        assert_eq!(result, dir.join("hello_1.txt"));
    }

    #[test]
    fn test_deconflict_path_no_extension() {
        let dir = tempdir();
        fs::write(dir.join("Makefile"), b"").unwrap();
        let name = std::ffi::OsStr::new("Makefile");
        let result = deconflict_path(&dir, name).unwrap();
        assert_eq!(result, dir.join("Makefile_1"));
    }

    // ── helpers ────────────────────────────────────────────────────────────────

    fn tempdir() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "agentmux_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }
}
