// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wave file utility: conversion between WaveFile (storage) and FileInfo (RPC).
//! Port of Go's pkg/util/wavefileutil/wavefileutil.go.
//!
//! Provides the bridge between the internal file storage representation
//! and the wire format used by the RPC protocol.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use super::storage::filestore::{FileMeta, FileOpts, WaveFile};

/// URL pattern for wave file paths.
pub const FILE_PATH_PATTERN: &str = "wavefile://";

/// File information as exposed over the RPC wire.
/// Matches Go's `wshrpc.FileInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    /// Full path (e.g., `wavefile://zone-id/filename`).
    pub path: String,

    /// File name (last component of path).
    pub name: String,

    /// File size in bytes.
    pub size: i64,

    /// Modification timestamp (milliseconds since epoch).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub modts: i64,

    /// Whether this is a directory.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isdir: bool,

    /// MIME type (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,

    /// File options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opts: Option<FileOpts>,

    /// File metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<FileMeta>,

    /// Whether this is a readable file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub readonly: bool,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// Convert a WaveFile to FileInfo for RPC transmission.
pub fn wave_file_to_file_info(file: &WaveFile) -> FileInfo {
    let path = format!(
        "{}{}/{}",
        FILE_PATH_PATTERN, file.zoneid, file.name
    );
    FileInfo {
        path,
        name: file.name.clone(),
        size: file.size,
        modts: file.modts,
        isdir: false,
        mimetype: None,
        opts: Some(file.opts.clone()),
        meta: if file.meta.is_empty() {
            None
        } else {
            Some(file.meta.clone())
        },
        readonly: false,
    }
}

/// Convert a list of WaveFiles to FileInfo list.
pub fn wave_file_list_to_file_info_list(files: &[WaveFile]) -> Vec<FileInfo> {
    files.iter().map(wave_file_to_file_info).collect()
}

/// Parse a wave file path into (zone_id, file_name).
/// Expects format: `wavefile://zone-id/filename`
pub fn parse_wave_file_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix(FILE_PATH_PATTERN)?;
    let slash_pos = rest.find('/')?;
    let zone_id = &rest[..slash_pos];
    let name = &rest[slash_pos + 1..];
    if zone_id.is_empty() || name.is_empty() {
        return None;
    }
    Some((zone_id.to_string(), name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_test_file() -> WaveFile {
        WaveFile {
            zoneid: "zone-abc".to_string(),
            name: "test.txt".to_string(),
            size: 1024,
            createdts: 1700000000000,
            modts: 1700000001000,
            opts: FileOpts::default(),
            meta: HashMap::new(),
        }
    }

    #[test]
    fn test_wave_file_to_file_info() {
        let file = make_test_file();
        let info = wave_file_to_file_info(&file);
        assert_eq!(info.path, "wavefile://zone-abc/test.txt");
        assert_eq!(info.name, "test.txt");
        assert_eq!(info.size, 1024);
        assert_eq!(info.modts, 1700000001000);
        assert!(!info.isdir);
        assert!(!info.readonly);
        assert!(info.meta.is_none()); // empty map → None
    }

    #[test]
    fn test_wave_file_to_file_info_with_meta() {
        let mut file = make_test_file();
        file.meta
            .insert("custom".to_string(), serde_json::json!("value"));
        let info = wave_file_to_file_info(&file);
        assert!(info.meta.is_some());
        assert_eq!(
            info.meta.unwrap().get("custom"),
            Some(&serde_json::json!("value"))
        );
    }

    #[test]
    fn test_wave_file_to_file_info_with_opts() {
        let mut file = make_test_file();
        file.opts.circular = true;
        file.opts.maxsize = 65536;
        let info = wave_file_to_file_info(&file);
        let opts = info.opts.unwrap();
        assert!(opts.circular);
        assert_eq!(opts.maxsize, 65536);
    }

    #[test]
    fn test_wave_file_list_to_file_info_list() {
        let files = vec![make_test_file(), make_test_file()];
        let infos = wave_file_list_to_file_info_list(&files);
        assert_eq!(infos.len(), 2);
    }

    #[test]
    fn test_wave_file_list_empty() {
        let infos = wave_file_list_to_file_info_list(&[]);
        assert!(infos.is_empty());
    }

    #[test]
    fn test_file_info_serialization() {
        let info = FileInfo {
            path: "wavefile://z1/f1".to_string(),
            name: "f1".to_string(),
            size: 100,
            modts: 0,
            isdir: false,
            mimetype: None,
            opts: None,
            meta: None,
            readonly: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        // Zero modts should be skipped
        assert!(!json.contains("modts"));
        // False isdir should be skipped
        assert!(!json.contains("isdir"));
        // None opts should be skipped
        assert!(!json.contains("opts"));
    }

    #[test]
    fn test_file_info_deserialization() {
        let json = r#"{"path":"wavefile://z/f","name":"f","size":50,"modts":123,"isdir":false}"#;
        let info: FileInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.path, "wavefile://z/f");
        assert_eq!(info.size, 50);
        assert_eq!(info.modts, 123);
    }

    #[test]
    fn test_parse_wave_file_path() {
        let result = parse_wave_file_path("wavefile://zone-abc/test.txt");
        assert_eq!(result, Some(("zone-abc".to_string(), "test.txt".to_string())));
    }

    #[test]
    fn test_parse_wave_file_path_nested() {
        let result = parse_wave_file_path("wavefile://zone/dir/file.txt");
        assert_eq!(
            result,
            Some(("zone".to_string(), "dir/file.txt".to_string()))
        );
    }

    #[test]
    fn test_parse_wave_file_path_invalid() {
        assert!(parse_wave_file_path("").is_none());
        assert!(parse_wave_file_path("file:///tmp/foo").is_none());
        assert!(parse_wave_file_path("wavefile://").is_none());
        assert!(parse_wave_file_path("wavefile://zone/").is_none());
        assert!(parse_wave_file_path("wavefile:///file").is_none());
    }
}
