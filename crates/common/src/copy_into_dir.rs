//! The one "copy a file or folder into a directory" used by drag and drop,
//! paste, and the attachment store's copy-to-workdir.
//! docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.5.
//!
//! Every destination is created exclusively (`create_new`, `create_dir`), so
//! two copies of `report.pdf` land as `report.pdf` and `report_1.pdf` instead
//! of racing for one name.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_NAME_CHARS: usize = 200;
const MAX_DECONFLICT: u32 = 1000;
const BUF_BYTES: usize = 1 << 20;

#[derive(Default)]
pub struct CopyControl<'a> {
    /// Called with the bytes copied so far.
    pub progress: Option<&'a dyn Fn(u64)>,
    /// Checked between chunks and entries; the partial copy is removed.
    pub cancel: Option<&'a AtomicBool>,
}

/// A source file name made valid as one path component on this platform,
/// keeping everything that already is, dotfiles included.
pub fn copy_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || c == '/' || c == '\\' || invalid_on_windows(c) { '_' } else { c })
        .collect();
    let trimmed = if cfg!(windows) { cleaned.trim_end_matches(['.', ' ']) } else { cleaned.as_str() };
    let short: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    if short.is_empty() || short.chars().all(|c| c == '.') {
        return "file".to_string();
    }
    if cfg!(windows) && is_reserved_on_windows(&short) {
        let (stem, ext) = split_ext(&short);
        return format!("{stem}_{ext}");
    }
    short
}

fn invalid_on_windows(c: char) -> bool {
    cfg!(windows) && matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

fn is_reserved_on_windows(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
}

/// `report.pdf` → (`report`, `.pdf`); a leading dot is part of the stem, so
/// `.env` → (`.env`, ``).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

fn candidate(dir: &Path, name: &str, n: u32) -> PathBuf {
    if n == 0 {
        return dir.join(name);
    }
    let (stem, ext) = split_ext(name);
    dir.join(format!("{stem}_{n}{ext}"))
}

fn cancelled(ctl: &CopyControl) -> bool {
    ctl.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

fn interrupted() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "copy cancelled")
}

/// Copy `src` (a file, or a folder recursively) into `dir` under `name` made
/// valid by [`copy_name`] and de-conflicted. Returns the path it landed at.
/// Symlinks inside a copied folder are skipped, so a link loop can't recurse.
/// On error or cancel nothing is left behind.
pub fn copy_into_dir(src: &Path, dir: &Path, name: &str, ctl: &CopyControl) -> std::io::Result<PathBuf> {
    if !dir.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("not a folder: {}", dir.display())));
    }
    let meta = std::fs::metadata(src)?;
    let name = copy_name(name);
    let mut copied = 0u64;
    if meta.is_dir() {
        let dest = create_unique(dir, &name, |p| std::fs::create_dir(p).map(|_| ()))?;
        match copy_dir_contents(src, &dest, ctl, &mut copied) {
            Ok(()) => Ok(dest),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                Err(e)
            }
        }
    } else {
        copy_file_unique(src, dir, &name, ctl, &mut copied)
    }
}

fn create_unique(dir: &Path, name: &str, create: impl Fn(&Path) -> std::io::Result<()>) -> std::io::Result<PathBuf> {
    for n in 0..MAX_DECONFLICT {
        let path = candidate(dir, name, n);
        match create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free name for {name} in {}", dir.display()),
    ))
}

fn copy_file_unique(src: &Path, dir: &Path, name: &str, ctl: &CopyControl, copied: &mut u64) -> std::io::Result<PathBuf> {
    let mut input = std::fs::File::open(src)?;
    for n in 0..MAX_DECONFLICT {
        let path = candidate(dir, name, n);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut out) => {
                let result = stream(&mut input, &mut out, ctl, copied);
                drop(out);
                return match result {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free name for {name} in {}", dir.display()),
    ))
}

fn stream(input: &mut impl Read, out: &mut impl Write, ctl: &CopyControl, copied: &mut u64) -> std::io::Result<()> {
    let mut buf = vec![0u8; BUF_BYTES];
    loop {
        if cancelled(ctl) {
            return Err(interrupted());
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            return out.flush();
        }
        out.write_all(&buf[..n])?;
        *copied += n as u64;
        if let Some(progress) = ctl.progress {
            progress(*copied);
        }
    }
}

fn copy_dir_contents(src: &Path, dest: &Path, ctl: &CopyControl, copied: &mut u64) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        if cancelled(ctl) {
            return Err(interrupted());
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            let sub = create_unique(dest, &copy_name(&name), |p| std::fs::create_dir(p).map(|_| ()))?;
            copy_dir_contents(&entry.path(), &sub, ctl, copied)?;
        } else if kind.is_file() {
            copy_file_unique(&entry.path(), dest, &copy_name(&name), ctl, copied)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn dirs() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn file(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn a_dotfile_keeps_its_name_and_deconflicts_after_it() {
        let (src_tmp, dst_tmp) = dirs();
        let (src, dst) = (src_tmp.path().to_path_buf(), dst_tmp.path().to_path_buf());
        let env = file(&src, ".env", "A=1");
        let first = copy_into_dir(&env, &dst, ".env", &CopyControl::default()).unwrap();
        let second = copy_into_dir(&env, &dst, ".env", &CopyControl::default()).unwrap();
        assert_eq!(first.file_name().unwrap(), ".env");
        assert_eq!(second.file_name().unwrap(), ".env_1");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "A=1");
    }

    #[test]
    fn extensions_stay_last_when_deconflicting() {
        let (src_tmp, dst_tmp) = dirs();
        let (src, dst) = (src_tmp.path().to_path_buf(), dst_tmp.path().to_path_buf());
        let f = file(&src, "report.pdf", "x");
        for _ in 0..3 {
            copy_into_dir(&f, &dst, "report.pdf", &CopyControl::default()).unwrap();
        }
        assert_eq!(names(&dst), vec!["report.pdf", "report_1.pdf", "report_2.pdf"]);
    }

    #[test]
    fn racing_copies_never_share_a_name() {
        let (src_tmp, dst_tmp) = dirs();
        let (src, dst) = (src_tmp.path().to_path_buf(), dst_tmp.path().to_path_buf());
        let f = file(&src, "same.txt", "x");
        let (f, dst2) = (Arc::new(f), Arc::new(dst.clone()));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (f, d) = (Arc::clone(&f), Arc::clone(&dst2));
                std::thread::spawn(move || copy_into_dir(&f, &d, "same.txt", &CopyControl::default()).unwrap())
            })
            .collect();
        let mut landed: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        landed.sort();
        landed.dedup();
        assert_eq!(landed.len(), 8);
        assert_eq!(names(&dst).len(), 8);
    }

    #[test]
    fn folders_copy_recursively_and_deconflict_as_a_whole() {
        let (src_tmp, dst_tmp) = dirs();
        let (src, dst) = (src_tmp.path().to_path_buf(), dst_tmp.path().to_path_buf());
        let proj = src.join("proj");
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        file(&proj, "a.txt", "a");
        file(&proj.join("sub"), ".hidden", "h");
        let first = copy_into_dir(&proj, &dst, "proj", &CopyControl::default()).unwrap();
        let second = copy_into_dir(&proj, &dst, "proj", &CopyControl::default()).unwrap();
        assert_eq!(names(&dst), vec!["proj", "proj_1"]);
        assert_eq!(std::fs::read_to_string(first.join("sub").join(".hidden")).unwrap(), "h");
        assert_eq!(std::fs::read_to_string(second.join("a.txt")).unwrap(), "a");
    }

    #[test]
    fn a_cancelled_copy_leaves_nothing_behind() {
        let (src_tmp, dst_tmp) = dirs();
        let (src, dst) = (src_tmp.path().to_path_buf(), dst_tmp.path().to_path_buf());
        let f = file(&src, "big.bin", &"x".repeat(3 * BUF_BYTES));
        let cancel = AtomicBool::new(false);
        let trip = |n: u64| {
            if n >= BUF_BYTES as u64 {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let ctl = CopyControl { progress: Some(&trip), cancel: Some(&cancel) };
        let err = copy_into_dir(&f, &dst, "big.bin", &ctl).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        assert!(names(&dst).is_empty());

        let proj = src.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        file(&proj, "a.txt", "a");
        let stopped = AtomicBool::new(true);
        let ctl = CopyControl { progress: None, cancel: Some(&stopped) };
        assert!(copy_into_dir(&proj, &dst, "proj", &ctl).is_err());
        assert!(names(&dst).is_empty(), "the partial folder is removed");
    }

    #[test]
    fn progress_reports_bytes_copied() {
        let (src_tmp, dst_tmp) = dirs();
        let (src, dst) = (src_tmp.path().to_path_buf(), dst_tmp.path().to_path_buf());
        let f = file(&src, "p.bin", &"y".repeat(BUF_BYTES + 10));
        let last = std::cell::Cell::new(0u64);
        let see = |n: u64| last.set(n);
        copy_into_dir(&f, &dst, "p.bin", &CopyControl { progress: Some(&see), cancel: None }).unwrap();
        assert_eq!(last.get(), (BUF_BYTES + 10) as u64);
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let src_tmp = tempfile::tempdir().unwrap();
        let src = src_tmp.path().to_path_buf();
        let f = file(&src, "a.txt", "a");
        assert!(copy_into_dir(&f, &src.join("nope"), "a.txt", &CopyControl::default()).is_err());
    }

    #[test]
    fn copy_name_keeps_valid_names_and_fixes_invalid_ones() {
        assert_eq!(copy_name(".env"), ".env");
        assert_eq!(copy_name("notes.md"), "notes.md");
        assert_eq!(copy_name("a/b\\c"), "a_b_c");
        assert_eq!(copy_name("tab\there"), "tab_here");
        assert_eq!(copy_name(""), "file");
        assert_eq!(copy_name(".."), "file");
        assert_eq!(copy_name("."), "file");
        assert_eq!(copy_name(&"n".repeat(500)).chars().count(), MAX_NAME_CHARS);
        if cfg!(windows) {
            assert_eq!(copy_name("what?.txt"), "what_.txt");
            assert_eq!(copy_name("a:b*c|d"), "a_b_c_d");
            assert_eq!(copy_name("trailing. "), "trailing");
            assert_eq!(copy_name("CON"), "CON_");
            assert_eq!(copy_name("nul.txt"), "nul_.txt");
            assert_eq!(copy_name("com1.log"), "com1_.log");
            assert_eq!(copy_name("COM0.log"), "COM0.log");
            assert_eq!(copy_name("console.txt"), "console.txt");
        } else {
            assert_eq!(copy_name("what?.txt"), "what?.txt");
            assert_eq!(copy_name("trailing. "), "trailing. ");
        }
    }
}
