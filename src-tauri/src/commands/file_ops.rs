// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// File operations for drag & drop support.

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
    // Normalize forward slashes — OSC 7 emits forward-slash paths on Windows (e.g. C:/Users/foo)
    let target_dir_norm = target_dir.replace('/', std::path::MAIN_SEPARATOR_STR);
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
