// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{Migration, MigrationContext, MigrationError, MigrationScope};
use std::path::Path;

pub struct M0001LegacyDataDir;

impl Migration for M0001LegacyDataDir {
    fn id(&self) -> &'static str { "0001_legacy_data_dir" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Move ~/.waveterm data directory to ~/.agentmux" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        // Runner always creates data_dir/db/ before migrations run, so
        // data_dir.exists() is always true and cannot guard against overwriting
        // a real agentmux install. Use agents/ instead — runner never creates it.
        if ctx.home.join("agents").exists() {
            return Ok(());
        }
        let home_dir = dirs::home_dir()
            .ok_or_else(|| MigrationError("cannot resolve OS home dir".to_owned()))?;
        let old_dir = home_dir.join(".waveterm");
        if !old_dir.exists() {
            return Ok(());
        }
        copy_dir_all(&old_dir, &ctx.data_dir)
            .map_err(|e| MigrationError(format!("legacy data dir copy: {}", e)))
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("cannot create {}: {}", dst.display(), e))?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| format!("cannot read {}: {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("read_dir entry: {}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("copy {}: {}", src_path.display(), e))?;
        }
    }
    Ok(())
}
