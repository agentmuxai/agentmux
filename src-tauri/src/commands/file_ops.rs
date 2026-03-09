// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// File operations for drag & drop support.

#[tauri::command]
pub async fn copy_file_to_dir(
    source_path: String,
    target_dir: String,
) -> Result<String, String> {
    let source = std::path::Path::new(&source_path);
    let target_dir = std::path::Path::new(&target_dir);

    if !source.exists() {
        return Err(format!("Source file not found: {}", source.display()));
    }

    if !source.is_file() {
        return Err(format!("Source is not a file: {}", source.display()));
    }

    if !target_dir.exists() {
        return Err(format!("Target directory not found: {}", target_dir.display()));
    }

    if !target_dir.is_dir() {
        return Err(format!("Target path is not a directory: {}", target_dir.display()));
    }

    let file_name = source
        .file_name()
        .ok_or_else(|| "Invalid source path".to_string())?;
    let target = target_dir.join(file_name);

    if target.exists() {
        return Err(format!("File already exists: {}", target.display()));
    }

    std::fs::copy(source, &target)
        .map_err(|e| format!("Copy failed: {}", e))?;

    Ok(target.display().to_string())
}
