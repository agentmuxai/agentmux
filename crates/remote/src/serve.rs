// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-remote serve --stdio`: file operations for srv over this
//! process's stdin and stdout (`fsproto`), one request at a time, until
//! stdin closes (spec §6.1, §6.3). Paths are the host's own; `~` and a
//! relative path are under the user's home. Nothing here opens a socket or
//! reaches beyond what this user can already read and write.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use crate::fsproto::{self, Entry, ErrKind, Kind, Reply, Request, Splitter, MAX_READ, MAX_WRITE};

/// Serve requests from `input`, answering on `output`, until `input` ends.
/// `home` is the user's home directory ([`home_dir`]).
pub fn run(home: &Path, input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    let mut server = Server::new(home.to_path_buf());
    let mut splitter = Splitter::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        let bodies = splitter
            .push(&buf[..n])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        for body in bodies {
            let reply = match Request::decode(&body) {
                Ok((id, req)) => (id, server.handle(req)),
                // A request that cannot be read: the stream is broken.
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };
            output.write_all(&reply.1.encode(reply.0))?;
            output.flush()?;
        }
    }
}

/// One connection's server: [`handle`], plus what lasts between requests
/// (Tower's CPU rates, `procs`).
pub struct Server {
    home: PathBuf,
    procs: crate::procs::Sampler,
}

impl Server {
    pub fn new(home: PathBuf) -> Self {
        Self { home, procs: crate::procs::Sampler::new() }
    }

    pub fn handle(&mut self, req: Request) -> Reply {
        match req {
            Request::Procs { top, filter } => Reply::Procs(self.procs.frame(top, &filter)),
            req => handle(&self.home, req),
        }
    }
}

pub fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// A request's path on this host: `~` and `~/…` under the home directory,
/// a relative path too, an absolute one as given.
pub fn resolve(home: &Path, path: &str) -> PathBuf {
    let p = if path == "~" {
        home.to_path_buf()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        home.join(path)
    };
    // Rebuilt from its parts: no trailing separator, which would make the OS
    // follow a final symlink (`~/link/` is the link's target, not the link)
    // and act on something other than what was named.
    p.components().collect()
}

fn err(e: io::Error) -> Reply {
    Reply::Err {
        kind: ErrKind::from_io(&e),
        message: e.to_string(),
    }
}

fn invalid(message: &str) -> Reply {
    Reply::Err {
        kind: ErrKind::Invalid,
        message: message.to_string(),
    }
}

/// The reply to one request; `home` is the user's home directory. A `Procs`
/// here has no previous round, so its frame has no CPU rates: the serve loop
/// answers it through [`Server`] instead.
pub fn handle(home: &Path, req: Request) -> Reply {
    let result = match req {
        Request::Procs { top, filter } => Ok(Reply::Procs(crate::procs::Sampler::new().frame(top, &filter))),
        Request::Hello => Ok(Reply::Hello {
            protocol: fsproto::PROTOCOL,
            home: home.to_string_lossy().into_owned(),
            os: std::env::consts::OS.to_string(),
        }),
        Request::Stat { path } => {
            let p = resolve(home, &path);
            entry_of(&p).map(Reply::Stat)
        }
        Request::List {
            path,
            offset,
            limit,
        } => list(&resolve(home, &path), offset, limit),
        Request::Read { path, offset, len } => read(&resolve(home, &path), offset, len),
        Request::Write { path, data } => {
            if data.len() > MAX_WRITE {
                return Reply::Err {
                    kind: ErrKind::TooLarge,
                    message: format!("over the {} MB a write may be", MAX_WRITE >> 20),
                };
            }
            write_atomic(&resolve(home, &path), &data).map(|()| Reply::Done)
        }
        Request::Append { path, data } => {
            if data.len() > MAX_WRITE {
                return Reply::Err {
                    kind: ErrKind::TooLarge,
                    message: format!("over the {} MB a write may be", MAX_WRITE >> 20),
                };
            }
            let p = resolve(home, &path);
            // Only a file already there: never makes one, never follows a
            // link somewhere else.
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_file() => fs::OpenOptions::new()
                    .append(true)
                    .open(&p)
                    .and_then(|mut f| f.write_all(&data))
                    .map(|()| Reply::Done),
                Ok(_) => return invalid("only a regular file can be appended to"),
                Err(e) => Err(e),
            }
        }
        Request::Mkdir { path, parents } => {
            let p = resolve(home, &path);
            if parents {
                fs::create_dir_all(&p)
            } else {
                fs::create_dir(&p)
            }
            .map(|()| Reply::Done)
        }
        Request::Rename { from, to } => {
            let (from, to) = (resolve(home, &from), resolve(home, &to));
            if let Err(e) = fs::symlink_metadata(&from) {
                return err(e);
            }
            // Never over something already there: a rename that would
            // replace a file says so instead. The same file under another
            // spelling (a case-only rename on a case-insensitive disk) is
            // not something else.
            if fs::symlink_metadata(&to).is_ok() && !same_file(&from, &to) {
                return Reply::Err {
                    kind: ErrKind::AlreadyExists,
                    message: format!("{} already exists", to.display()),
                };
            }
            fs::rename(&from, &to).map(|()| Reply::Done)
        }
        Request::Replace { from, to } => {
            let (from, to) = (resolve(home, &from), resolve(home, &to));
            // Files only: `rename` over a file is atomic, and a folder is
            // never replaced this way.
            for p in [&from, &to] {
                match fs::symlink_metadata(p) {
                    Ok(m) if m.is_dir() => return invalid("only a file is replaced in one step"),
                    Ok(_) => {}
                    Err(e) => return err(e),
                }
            }
            fs::rename(&from, &to).map(|()| Reply::Done)
        }
        Request::Sync { path } => {
            let p = resolve(home, &path);
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_file() => fs::OpenOptions::new()
                    // Read-only on Unix (fsync needs no write access, and a read-only file
                    // must sync too); Windows flushes only a handle it can write.
                    .read(cfg!(unix))
                    .write(!cfg!(unix))
                    .open(&p)
                    .and_then(|f| f.sync_all())
                    .map(|()| Reply::Done),
                Ok(_) => return invalid("only a file is synced"),
                Err(e) => Err(e),
            }
        }
        Request::Realpath { path } => fs::canonicalize(resolve(home, &path))
            .map(|p| Reply::Path(p.to_string_lossy().into_owned())),
        Request::SetMeta {
            path,
            mode,
            mtime_ms,
        } => {
            let p = resolve(home, &path);
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_file() => set_meta(&p, mode, mtime_ms).map(|()| Reply::Done),
                Ok(_) => return invalid("only a file's metadata is set"),
                Err(e) => Err(e),
            }
        }
        Request::Delete { path, recursive } => {
            let p = resolve(home, &path);
            // `..` is never needed to name a thing to delete, and it is how a
            // path that looks like it is under home reaches home or above.
            if p.components().any(|c| c == std::path::Component::ParentDir) {
                return invalid("refusing to delete a path with '..' in it");
            }
            // What it really is, its parents' links resolved (the thing
            // itself is not followed: a link is removed, never its target).
            let real = match (p.parent(), p.file_name()) {
                (Some(parent), Some(name)) => match fs::canonicalize(parent) {
                    Ok(dir) => dir.join(name),
                    Err(e) => return err(e),
                },
                _ => return invalid("refusing to delete the root"),
            };
            let real_home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
            if real.parent().is_none() || real == real_home || real_home.starts_with(&real) {
                return invalid(
                    "refusing to delete the root, the home directory or a folder holding it",
                );
            }
            // Everything from here on uses `real`, the path the guard checked:
            // never `p`, whose trailing slash (`~/link/`) would make the OS
            // follow a link the guard saw as a link.
            match fs::symlink_metadata(&real) {
                // A link is removed, never followed (a directory link is a
                // directory entry on Windows).
                Ok(m) if m.file_type().is_symlink() => {
                    fs::remove_file(&real).or_else(|_| fs::remove_dir(&real))
                }
                Ok(m) if m.is_dir() => {
                    if recursive {
                        fs::remove_dir_all(&real)
                    } else {
                        fs::remove_dir(&real)
                    }
                }
                Ok(_) => fs::remove_file(&real),
                Err(e) => Err(e),
            }
            .map(|()| Reply::Done)
        }
    };
    result.unwrap_or_else(err)
}

/// A file's permission bits (`mode`, when not 0, on Unix) and modification
/// time (`mtime_ms`, when not 0).
fn set_meta(p: &Path, mode: u32, mtime_ms: i64) -> io::Result<()> {
    if mtime_ms > 0 {
        let t = UNIX_EPOCH + std::time::Duration::from_millis(mtime_ms as u64);
        fs::OpenOptions::new()
            .write(true)
            .open(p)?
            .set_modified(t)?;
    }
    #[cfg(unix)]
    if mode != 0 {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(mode & 0o7777))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

fn entry_of(p: &Path) -> io::Result<Entry> {
    let link = fs::symlink_metadata(p)?;
    let symlink = link.file_type().is_symlink();
    // A symlink is what it points to; a dangling one is shown as itself.
    let meta = if symlink {
        fs::metadata(p).unwrap_or(link)
    } else {
        link
    };
    let kind = if meta.is_dir() {
        Kind::Dir
    } else if meta.is_file() {
        Kind::File
    } else {
        Kind::Other
    };
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        // The permission bits only, not the file type in st_mode.
        meta.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = 0;
    let link_target = if symlink {
        fs::read_link(p)
            .map(|t| t.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(Entry {
        link_target,
        name: p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned()),
        kind,
        size: meta.len(),
        mtime_ms,
        mode,
        symlink,
    })
}

/// Whether `a` and `b` are one file (both exist): the same inode on Unix,
/// the same final path on Windows, which gives each file one spelling.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (fs::canonicalize(a), fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}

fn list(dir: &Path, offset: u32, limit: u32) -> io::Result<Reply> {
    let mut names: Vec<std::ffi::OsString> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    names.sort();
    let total = names.len() as u32;
    let entries = names
        .into_iter()
        .skip(offset as usize)
        .take(limit.min(5000) as usize)
        // An entry that vanished between the listing and its stat is left out.
        .filter_map(|n| entry_of(&dir.join(n)).ok())
        .collect();
    Ok(Reply::List { entries, total })
}

fn read(p: &Path, offset: u64, len: u32) -> io::Result<Reply> {
    use std::io::{Seek, SeekFrom};
    // Never stuck opening a pipe: whatever this opened must be a regular
    // file. A link is read through, as the editor opens a linked dotfile
    // (a transfer skips links before it reads).
    #[cfg(unix)]
    let mut f = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(p)?
    };
    #[cfg(not(unix))]
    let mut f = fs::File::open(p)?;
    let opened = f.metadata()?;
    if !opened.is_dir() && !opened.is_file() {
        return Ok(Reply::Err {
            kind: ErrKind::Invalid,
            message: format!("{} isn't a regular file", p.display()),
        });
    }
    if opened.is_dir() {
        return Ok(Reply::Err {
            kind: ErrKind::IsADirectory,
            message: format!("{} is a directory", p.display()),
        });
    }
    f.seek(SeekFrom::Start(offset))?;
    let want = len.min(MAX_READ) as usize;
    let mut data = Vec::with_capacity(want);
    Read::by_ref(&mut f)
        .take(want as u64)
        .read_to_end(&mut data)?;
    // At the end if this range reached it.
    let eof = data.len() < want || f.read(&mut [0u8; 1])? == 0;
    Ok(Reply::Read { data, eof })
}

/// Replace `p` with `data`: write a temp file beside it, sync it, give it the
/// old file's permissions, and rename it over. A reader never sees half a
/// file, and a failure leaves the old one as it was. A symlink is written
/// through to its target, so the link stays a link.
fn write_atomic(p: &Path, data: &[u8]) -> io::Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let target = match fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(p)?,
        _ => p.to_path_buf(),
    };
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?;
    let tmp = dir.join(format!(
        ".{}.amx-{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let old = fs::metadata(&target).ok();
    if old.as_ref().is_some_and(|m| m.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a directory", target.display()),
        ));
    }
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        if let Some(m) = &old {
            fs::set_permissions(&tmp, m.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run requests through `run` as srv would, and return the replies.
    fn exchange(home: &Path, requests: &[Request]) -> Vec<Reply> {
        let mut input = Vec::new();
        for (i, r) in requests.iter().enumerate() {
            input.extend(r.encode(i as u32));
        }
        let mut output = Vec::new();
        run(home, &mut input.as_slice(), &mut output).unwrap();
        let mut s = Splitter::new();
        s.push(&output)
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (id, reply) = Reply::decode(b).unwrap();
                assert_eq!(id, i as u32, "replies in order, each with its id");
                reply
            })
            .collect()
    }

    #[test]
    fn files_are_listed_read_written_renamed_and_deleted() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let replies = exchange(
            h,
            &[
                Request::Hello,
                Request::Mkdir {
                    path: "~/d/e".into(),
                    parents: true,
                },
                Request::Write {
                    path: "d/a.txt".into(),
                    data: b"hello world".to_vec(),
                },
                Request::Read {
                    path: "~/d/a.txt".into(),
                    offset: 6,
                    len: 100,
                },
                Request::Read {
                    path: "~/d/a.txt".into(),
                    offset: 0,
                    len: 5,
                },
                Request::List {
                    path: "~/d".into(),
                    offset: 0,
                    limit: 10,
                },
                Request::List {
                    path: "~/d".into(),
                    offset: 1,
                    limit: 10,
                },
                Request::Rename {
                    from: "~/d/a.txt".into(),
                    to: "~/d/b.txt".into(),
                },
                Request::Stat {
                    path: "~/d/a.txt".into(),
                },
                Request::Delete {
                    path: "~/d".into(),
                    recursive: false,
                },
                Request::Delete {
                    path: "~/d".into(),
                    recursive: true,
                },
                Request::Stat { path: "~/d".into() },
            ],
        );
        match &replies[0] {
            Reply::Hello {
                protocol, home: hh, ..
            } => {
                assert_eq!(*protocol, fsproto::PROTOCOL);
                assert_eq!(Path::new(hh), h);
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(replies[1], Reply::Done);
        assert_eq!(replies[2], Reply::Done);
        assert_eq!(
            replies[3],
            Reply::Read {
                data: b"world".to_vec(),
                eof: true
            }
        );
        assert_eq!(
            replies[4],
            Reply::Read {
                data: b"hello".to_vec(),
                eof: false
            }
        );
        match &replies[5] {
            Reply::List { entries, total } => {
                assert_eq!(*total, 2);
                let got: Vec<(&str, Kind, u64)> = entries
                    .iter()
                    .map(|e| (e.name.as_str(), e.kind, e.size))
                    .collect();
                assert_eq!(
                    got,
                    vec![("a.txt", Kind::File, 11), ("e", Kind::Dir, got[1].2)]
                );
                assert!(entries[0].mtime_ms > 0);
            }
            r => panic!("{r:?}"),
        }
        match &replies[6] {
            Reply::List { entries, total } => {
                assert_eq!(
                    (*total, entries.len(), entries[0].name.as_str()),
                    (2, 1, "e")
                )
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(replies[7], Reply::Done);
        assert!(matches!(
            replies[8],
            Reply::Err {
                kind: ErrKind::NotFound,
                ..
            }
        ));
        // Not empty, and not recursive: refused, kept.
        assert!(matches!(replies[9], Reply::Err { .. }), "{:?}", replies[9]);
        assert_eq!(replies[10], Reply::Done);
        assert!(matches!(
            replies[11],
            Reply::Err {
                kind: ErrKind::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn a_write_replaces_whole_keeps_the_mode_and_follows_a_link() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let f = h.join("f.sh");
        fs::write(&f, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&f, fs::Permissions::from_mode(0o750)).unwrap();
            std::os::unix::fs::symlink(&f, h.join("link")).unwrap();
        }
        let replies = exchange(
            h,
            &[
                Request::Write {
                    path: "~/f.sh".into(),
                    data: b"new".to_vec(),
                },
                #[cfg(unix)]
                Request::Write {
                    path: "~/link".into(),
                    data: b"via link".to_vec(),
                },
            ],
        );
        assert!(replies.iter().all(|r| *r == Reply::Done), "{replies:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::read(&f).unwrap(), b"via link");
            assert_eq!(
                fs::metadata(&f).unwrap().permissions().mode() & 0o777,
                0o750
            );
            assert!(fs::symlink_metadata(h.join("link"))
                .unwrap()
                .file_type()
                .is_symlink());
        }
        #[cfg(not(unix))]
        assert_eq!(fs::read(&f).unwrap(), b"new");
        // No temp file left behind.
        let left: Vec<_> = fs::read_dir(h)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
    }

    #[test]
    fn a_case_only_rename_is_not_refused_as_a_clash() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::write(h.join("a.txt"), b"a").unwrap();
        let case_insensitive = h.join("A.TXT").exists();
        let r = exchange(
            h,
            &[Request::Rename {
                from: "a.txt".into(),
                to: "A.txt".into(),
            }],
        );
        assert_eq!(
            r[0],
            Reply::Done,
            "case-insensitive disk: {case_insensitive}"
        );
        let names: Vec<String> = fs::read_dir(h)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["A.txt"]);
    }

    #[test]
    fn a_path_never_ends_in_a_separator() {
        let h = Path::new("/home/u");
        assert_eq!(resolve(h, "~/link/"), Path::new("/home/u/link"));
        assert_eq!(resolve(h, "/srv/data//"), Path::new("/srv/data"));
        assert_eq!(resolve(h, "rel/"), Path::new("/home/u/rel"));
        assert_eq!(resolve(h, "~"), h);
    }

    #[test]
    fn append_adds_to_an_existing_file_only() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let r = exchange(
            h,
            &[
                Request::Write {
                    path: "~/up".into(),
                    data: b"ab".to_vec(),
                },
                Request::Append {
                    path: "~/up".into(),
                    data: b"cd".to_vec(),
                },
                Request::Append {
                    path: "~/missing".into(),
                    data: b"x".to_vec(),
                },
                Request::Append {
                    path: "~".into(),
                    data: b"x".to_vec(),
                },
            ],
        );
        assert_eq!((&r[0], &r[1]), (&Reply::Done, &Reply::Done));
        assert!(
            matches!(
                r[2],
                Reply::Err {
                    kind: ErrKind::NotFound,
                    ..
                }
            ),
            "{:?}",
            r[2]
        );
        assert!(
            matches!(
                r[3],
                Reply::Err {
                    kind: ErrKind::Invalid,
                    ..
                }
            ),
            "{:?}",
            r[3]
        );
        assert_eq!(fs::read(h.join("up")).unwrap(), b"abcd");
        assert!(!h.join("missing").exists());
    }

    #[test]
    fn replace_realpath_and_set_meta() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::write(h.join("new"), b"new").unwrap();
        fs::write(h.join("old"), b"old").unwrap();
        fs::create_dir(h.join("d")).unwrap();
        let r = exchange(
            h,
            &[
                Request::Replace {
                    from: "~/new".into(),
                    to: "~/old".into(),
                },
                Request::Replace {
                    from: "~/old".into(),
                    to: "~/d".into(),
                },
                Request::Realpath {
                    path: "~/d/../old".into(),
                },
                Request::SetMeta {
                    path: "~/old".into(),
                    mode: 0o751,
                    mtime_ms: 1_600_000_000_000,
                },
                Request::SetMeta {
                    path: "~/d".into(),
                    mode: 0o700,
                    mtime_ms: 0,
                },
            ],
        );
        assert_eq!(r[0], Reply::Done);
        assert_eq!(fs::read(h.join("old")).unwrap(), b"new");
        assert!(!h.join("new").exists());
        assert!(
            matches!(
                r[1],
                Reply::Err {
                    kind: ErrKind::Invalid,
                    ..
                }
            ),
            "{:?}",
            r[1]
        );
        match &r[2] {
            Reply::Path(p) => assert_eq!(Path::new(p), fs::canonicalize(h.join("old")).unwrap()),
            other => panic!("{other:?}"),
        }
        assert_eq!(r[3], Reply::Done);
        let e = entry_of(&h.join("old")).unwrap();
        assert_eq!(e.mtime_ms, 1_600_000_000_000);
        #[cfg(unix)]
        assert_eq!(e.mode, 0o751);
        assert!(matches!(
            r[4],
            Reply::Err {
                kind: ErrKind::Invalid,
                ..
            }
        ));
    }

    /// A pipe is refused at once (never a read that waits for a writer); a
    /// link is read through.
    #[cfg(unix)]
    #[test]
    fn a_read_takes_only_a_regular_file() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        assert!(std::process::Command::new("mkfifo")
            .arg(h.join("pipe"))
            .status()
            .unwrap()
            .success());
        fs::write(h.join("real"), b"r").unwrap();
        std::os::unix::fs::symlink(h.join("real"), h.join("link")).unwrap();
        let r = exchange(
            h,
            &[
                Request::Read {
                    path: "~/pipe".into(),
                    offset: 0,
                    len: 10,
                },
                Request::Read {
                    path: "~/link".into(),
                    offset: 0,
                    len: 10,
                },
                Request::Sync {
                    path: "~/real".into(),
                },
            ],
        );
        assert!(
            matches!(
                r[0],
                Reply::Err {
                    kind: ErrKind::Invalid,
                    ..
                }
            ),
            "{:?}",
            r[0]
        );
        assert_eq!(
            r[1],
            Reply::Read {
                data: b"r".to_vec(),
                eof: true
            },
            "a link is read through"
        );
        assert_eq!(r[2], Reply::Done);
        // A read-only file syncs too.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(h.join("real"), fs::Permissions::from_mode(0o444)).unwrap();
        let r = exchange(
            h,
            &[Request::Sync {
                path: "~/real".into(),
            }],
        );
        assert_eq!(r[0], Reply::Done);
    }

    #[test]
    fn modes_are_permission_bits_only() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("f"), b"").unwrap();
        let e = entry_of(&home.path().join("f")).unwrap();
        assert_eq!(e.mode & !0o7777, 0, "{:o}", e.mode);
    }

    #[test]
    fn it_refuses_what_would_destroy_more_than_asked() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::write(h.join("a"), b"a").unwrap();
        fs::write(h.join("b"), b"b").unwrap();
        fs::create_dir(h.join("x")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(h, h.join("to-home")).unwrap();
            std::os::unix::fs::symlink(h, h.join("to-home2")).unwrap();
        }
        let replies = exchange(
            h,
            &[
                Request::Rename {
                    from: "a".into(),
                    to: "b".into(),
                },
                Request::Delete {
                    path: "~".into(),
                    recursive: true,
                },
                Request::Delete {
                    path: "/".into(),
                    recursive: true,
                },
                Request::Read {
                    path: "~".into(),
                    offset: 0,
                    len: 10,
                },
                // Home, or above it, spelled another way.
                Request::Delete {
                    path: "~/x/..".into(),
                    recursive: true,
                },
                Request::Delete {
                    path: "~/..".into(),
                    recursive: true,
                },
                Request::Delete {
                    path: h.join("x/..").to_string_lossy().into_owned(),
                    recursive: true,
                },
                Request::Rename {
                    from: "missing".into(),
                    to: "b".into(),
                },
            ],
        );
        assert!(matches!(
            replies[0],
            Reply::Err {
                kind: ErrKind::AlreadyExists,
                ..
            }
        ));
        for r in [
            &replies[1],
            &replies[2],
            &replies[4],
            &replies[5],
            &replies[6],
        ] {
            assert!(
                matches!(
                    r,
                    Reply::Err {
                        kind: ErrKind::Invalid,
                        ..
                    }
                ),
                "{r:?}"
            );
        }
        assert!(matches!(replies[3], Reply::Err { .. }));
        assert!(matches!(
            replies[7],
            Reply::Err {
                kind: ErrKind::NotFound,
                ..
            }
        ));
        assert_eq!(fs::read(h.join("b")).unwrap(), b"b");
        assert!(h.join("x").exists());

        // Through a link to home: the link goes, home stays.
        #[cfg(unix)]
        {
            let r = exchange(
                h,
                &[Request::Delete {
                    path: "~/to-home".into(),
                    recursive: true,
                }],
            );
            assert_eq!(r[0], Reply::Done);
            assert!(h.join("a").exists() && !h.join("to-home").exists());
            // A trailing slash would make the OS follow the link: still only
            // the link goes.
            let r = exchange(
                h,
                &[Request::Delete {
                    path: "~/to-home2/".into(),
                    recursive: true,
                }],
            );
            assert_eq!(r[0], Reply::Done);
            assert!(h.join("a").exists() && h.join("x").exists() && !h.join("to-home2").exists());
        }
    }
}
