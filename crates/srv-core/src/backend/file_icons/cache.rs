// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where finished icons are kept: a byte-bounded memory LRU, and a small disk
//! cache so a restart doesn't ask the shell again (spec §4.4). A type icon's
//! file name carries a hash of the file associations behind it, so changing a
//! default app makes a new file instead of needing a notification.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::request::IconRequest;

/// Memory kept for encoded icons.
pub const MEM_CAP_BYTES: usize = 32 * 1024 * 1024;
/// Disk kept for type icons (a few hundred small files).
pub const DISK_CAP_BYTES: u64 = 20 * 1024 * 1024;

/// Encoded PNGs by key, least recently used out first once over the cap.
#[derive(Default)]
pub struct MemCache {
    entries: HashMap<String, (Arc<Vec<u8>>, u64)>,
    bytes: usize,
    tick: u64,
}

impl MemCache {
    pub fn get(&mut self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(v, t)| {
            *t = tick;
            v.clone()
        })
    }

    pub fn put(&mut self, key: String, value: Arc<Vec<u8>>, cap: usize) {
        self.tick += 1;
        self.bytes += value.len();
        if let Some((old, _)) = self.entries.insert(key, (value, self.tick)) {
            self.bytes -= old.len();
        }
        while self.bytes > cap && self.entries.len() > 1 {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()) else { break };
            if let Some((v, _)) = self.entries.remove(&oldest) {
                self.bytes -= v.len();
            }
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The key a finished icon is kept under (memory and, hashed into the file name, disk).
pub fn key(req: &IconRequest, assoc: &str) -> String {
    format!("{}|{}|{}|{}", req.kind.as_str(), req.ext, req.px, assoc)
}

/// FNV-1a over the bytes: the same answer on every toolchain, since it names files that outlive a build.
pub fn stable_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `<root>/ext/<px>/<kind>-<ext>.<assoc>.png`. The extension is validated to
/// `[a-z0-9_+-]` and the kind prefix keeps a name like `con` from being a
/// reserved device name on Windows.
pub fn disk_path(root: &Path, req: &IconRequest, assoc: &str) -> PathBuf {
    root.join("ext").join(req.px.to_string()).join(format!("{}-{}.{}.png", req.kind.as_str(), req.ext, stable_hash(assoc)))
}

/// Delete the oldest files under `root` until what is left is at most `cap` bytes.
pub fn prune(root: &Path, cap: u64) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                files.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len(), entry.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::request::Kind;
    use super::*;

    fn req(ext: &str, px: u32) -> IconRequest {
        IconRequest { kind: Kind::File, ext: ext.to_string(), px }
    }

    #[test]
    fn the_least_recently_used_icon_leaves_first_and_a_read_counts_as_use() {
        let mut c = MemCache::default();
        let png = |n: usize| Arc::new(vec![0u8; n]);
        c.put("a".into(), png(40), 100);
        c.put("b".into(), png(40), 100);
        assert!(c.get("a").is_some(), "touch a, so b is now the oldest");
        c.put("c".into(), png(40), 100);
        assert!(c.get("b").is_none(), "b was evicted");
        assert!(c.get("a").is_some() && c.get("c").is_some());
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn replacing_an_entry_does_not_count_its_bytes_twice() {
        let mut c = MemCache::default();
        c.put("a".into(), Arc::new(vec![0; 60]), 100);
        c.put("a".into(), Arc::new(vec![0; 60]), 100);
        c.put("b".into(), Arc::new(vec![0; 30]), 100);
        assert_eq!(c.len(), 2, "60 + 30 fits in 100: nothing was evicted");
    }

    #[test]
    fn the_hash_is_stable_and_the_disk_name_changes_with_the_association() {
        assert_eq!(stable_hash(""), "cbf29ce484222325");
        assert_eq!(stable_hash("a"), "af63dc4c8601ec8c");
        let root = Path::new("cache");
        let a = disk_path(root, &req("rs", 16), "ProgA|icon1");
        let b = disk_path(root, &req("rs", 16), "ProgB|icon1");
        assert_ne!(a, b, "a new default app is a new file");
        assert_eq!(a, disk_path(root, &req("rs", 16), "ProgA|icon1"));
        assert!(a.ends_with(format!("file-rs.{}.png", stable_hash("ProgA|icon1"))));
        assert!(disk_path(root, &req("con", 16), "x").file_name().unwrap().to_string_lossy().starts_with("file-con."));
    }

    #[test]
    fn prune_removes_the_oldest_until_under_the_cap() {
        let dir = std::env::temp_dir().join(format!("agentmux-icons-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ext").join("16")).unwrap();
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            let p = dir.join("ext").join("16").join(name);
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
        }
        prune(&dir, 250);
        let left: Vec<String> = std::fs::read_dir(dir.join("ext").join("16"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!left.contains(&"old".to_string()), "the oldest went: {left:?}");
        assert_eq!(left.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
