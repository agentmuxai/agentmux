// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unpacking the Windows portable release's `.7z` in srv itself
//! (`docs/specs/SPEC_UPDATE_UNPACK_7Z_IN_RUST_2026_10_19.md`).
//!
//! The release is packed LZMA2 with the BCJ2 filter (`scripts/package-portable.sh`
//! runs `7z a -t7z -m0=lzma2 -mx=9`, and 7-Zip adds BCJ2 for executables).
//! Windows 10's own `tar.exe` is libarchive built without liblzma, so it can't
//! read one ("LZMA codec is unsupported"); `sevenz-rust2` decodes it here, on
//! any Windows. Every entry is written under the destination or refused: an
//! absolute path, a drive or root, or a `..` never reaches the disk.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// An entry's data, read until `stop` is set: a timed-out unpack must stop
/// writing, because its blocking thread can't be cancelled (QTest on #5124).
struct UntilStopped<'a> {
    inner: &'a mut dyn Read,
    stop: &'a AtomicBool,
}

impl Read for UntilStopped<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("unpacking was stopped"));
        }
        self.inner.read(buf)
    }
}

/// `dest` joined with an archive entry's `name`, or why it may not be
/// written. Both `/` and `\` separate, so a Windows-authored name is checked
/// the same on every platform. A `:` is refused too: on Windows `x:y` writes
/// an NTFS alternate data stream, and no release file has one.
fn entry_path(dest: &Path, name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    let mut path = dest.to_path_buf();
    let mut named = false;
    for part in Path::new(&normalized).components() {
        match part {
            Component::Normal(p) if p.to_string_lossy().contains(':') => {
                return Err(format!("refused an entry with a ':' in its name: {name}"));
            }
            Component::Normal(p) => {
                path.push(p);
                named = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("refused an entry outside the folder: {name}"));
            }
        }
    }
    if !named {
        return Err(format!("refused an entry with no name: {name:?}"));
    }
    Ok(path)
}

/// Unpacks the `.7z` at `archive` into `dest`, which exists. Blocking: run it
/// off the async runtime. Stops at the first entry it refuses or can't write,
/// or as soon as `stop` is set, leaving what it wrote for the caller's
/// staging folder to discard.
pub(crate) fn unpack_7z(archive: &Path, dest: &Path, stop: &AtomicBool) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("can't open {}: {e}", archive.display()))?;
    let mut reader = sevenz_rust2::ArchiveReader::new(std::io::BufReader::new(file), sevenz_rust2::Password::empty())
        .map_err(|e| format!("can't decode the archive: {e}"))?;
    // The callback can only return the crate's error; keep ours to report.
    let mut refused: Option<String> = None;
    let walked = reader.for_each_entries(|entry, data| {
        if stop.load(Ordering::Relaxed) {
            refused = Some("unpacking was stopped".to_string());
            return Err(sevenz_rust2::Error::Other("unpacking was stopped".into()));
        }
        let path = match entry_path(dest, entry.name()) {
            Ok(path) => path,
            Err(why) => {
                refused = Some(why.clone());
                return Err(sevenz_rust2::Error::Other(why.into()));
            }
        };
        if entry.is_directory() {
            std::fs::create_dir_all(&path)?;
            return Ok(true);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Always read the entry through: in a solid archive the next one
        // decodes from where this one ends.
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
        std::io::copy(&mut UntilStopped { inner: data, stop }, &mut out)?;
        out.flush()?;
        Ok(true)
    });
    match (walked, refused) {
        (_, Some(why)) => Err(why),
        (Err(e), None) => Err(format!("can't decode the archive: {e}")),
        (Ok(()), None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A portable-shaped folder packed with the release's own flags
    /// (`7z a -t7z -m0=lzma2 -mx=9 -ms=on`) plus `-mf=BCJ2`: 7-Zip only picks
    /// BCJ2 itself for a real PE, and these `.exe` files are small fakes. So the
    /// coder chain is the release's (`7z l -slt`: `LZMA2:14 LZMA:14 BCJ2`). The
    /// zip that used to stand in for the release hid that Windows' tar.exe
    /// can't read LZMA.
    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/backend/testdata/update-portable-0.59.27.7z");

    #[test]
    fn the_release_codecs_unpack_into_a_staged_portable() {
        let dest = tempfile::tempdir().unwrap();
        unpack_7z(Path::new(FIXTURE), dest.path(), &go()).unwrap();
        assert!(dest.path().join("agentmux.exe").is_file());
        assert!(dest.path().join("runtime/agentmux-portable.marker").is_file());
        assert!(dest.path().join("runtime/agentmux-0.59.27.exe").is_file());
        // The contents survive BCJ2 + LZMA2, not just the names.
        let exe = std::fs::read(dest.path().join("agentmux.exe")).unwrap();
        assert_eq!(&exe[..2], b"MZ");
        assert_eq!(std::fs::read_to_string(dest.path().join("runtime/README.txt")).unwrap().trim(), "AgentMux portable");
    }

    /// Not stopped.
    fn go() -> AtomicBool {
        AtomicBool::new(false)
    }

    // A timed-out unpack is told to stop, and writes nothing more (QTest on #5124).
    #[test]
    fn a_stopped_unpack_writes_nothing_more_and_says_so() {
        let dest = tempfile::tempdir().unwrap();
        let err = unpack_7z(Path::new(FIXTURE), dest.path(), &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err, "unpacking was stopped");
        assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
    }

    #[test]
    fn entries_outside_the_folder_are_refused() {
        let dest = Path::new("/tmp/stage");
        for name in ["../escape.txt", "a/../../escape.txt", "..\\escape.txt", "/etc/passwd", "\\abs.txt", ""] {
            assert!(entry_path(dest, name).is_err(), "{name:?} must be refused");
        }
        #[cfg(windows)]
        assert!(entry_path(dest, "C:\\Windows\\x.dll").is_err());
        // An NTFS alternate data stream on Windows; refused everywhere.
        assert!(entry_path(dest, "runtime/agentmux.exe:hidden").is_err());
        assert_eq!(entry_path(dest, "runtime/./agentmux.exe").unwrap(), dest.join("runtime").join("agentmux.exe"));
        assert_eq!(entry_path(dest, "runtime\\agentmux.exe").unwrap(), dest.join("runtime").join("agentmux.exe"));
    }

    #[test]
    fn a_truncated_archive_fails_with_a_reason_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = std::fs::read(FIXTURE).unwrap();
        let cut = dir.path().join("cut.7z");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let err = unpack_7z(&cut, &dir.path().join("out"), &go()).unwrap_err();
        assert!(err.starts_with("can't decode the archive"), "{err}");
        let err = unpack_7z(&dir.path().join("missing.7z"), dir.path(), &go()).unwrap_err();
        assert!(err.starts_with("can't open"), "{err}");
    }

    /// The real release, by hand: `AGENTMUX_TEST_PORTABLE_7Z=<agentmux-X-x64-portable.7z>
    /// AGENTMUX_TEST_PORTABLE_VERSION=X cargo test -p agentmux-srv real_release -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_release_unpacks_into_a_staged_portable() {
        let archive = std::env::var("AGENTMUX_TEST_PORTABLE_7Z").expect("AGENTMUX_TEST_PORTABLE_7Z");
        let version = std::env::var("AGENTMUX_TEST_PORTABLE_VERSION").expect("AGENTMUX_TEST_PORTABLE_VERSION");
        let dest = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        unpack_7z(Path::new(&archive), dest.path(), &go()).unwrap();
        eprintln!("unpacked in {:?}", started.elapsed());
        assert!(dest.path().join("agentmux.exe").is_file());
        assert!(dest.path().join("runtime/agentmux-portable.marker").is_file());
        assert!(dest.path().join("runtime").join(format!("agentmux-{version}.exe")).is_file());
        // `AGENTMUX_TEST_PORTABLE_COMPARE=<the same archive unpacked by 7-Zip>`:
        // every file byte for byte, so BCJ2 + LZMA2 decode exactly.
        if let Ok(reference) = std::env::var("AGENTMUX_TEST_PORTABLE_COMPARE") {
            let mut compared = 0;
            let mut stack = vec![PathBuf::from(&reference)];
            while let Some(dir) = stack.pop() {
                for e in std::fs::read_dir(&dir).unwrap() {
                    let p = e.unwrap().path();
                    if p.is_dir() {
                        stack.push(p);
                        continue;
                    }
                    let rel = p.strip_prefix(&reference).unwrap();
                    assert!(std::fs::read(&p).unwrap() == std::fs::read(dest.path().join(rel)).unwrap(), "{} differs", rel.display());
                    compared += 1;
                }
            }
            eprintln!("{compared} files identical to 7-Zip's");
        }
    }
}
