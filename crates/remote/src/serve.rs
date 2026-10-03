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

use agentmux_remote::fsproto::{
    self, Entry, ErrKind, Kind, Reply, Request, Splitter, MAX_READ, MAX_WRITE,
};

/// Serve requests from `input`, answering on `output`, until `input` ends.
/// `home` is the user's home directory ([`home_dir`]).
pub fn run(home: &Path, input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
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
                Ok((id, req)) => (id, handle(home, req)),
                // A request that cannot be read: the stream is broken.
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };
            output.write_all(&reply.1.encode(reply.0))?;
            output.flush()?;
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
    if path == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        home.join(p)
    }
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

fn handle(home: &Path, req: Request) -> Reply {
    let result = match req {
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
            // Never over something already there: a rename that would
            // replace a file says so instead.
            if fs::symlink_metadata(&to).is_ok() {
                return Reply::Err {
                    kind: ErrKind::AlreadyExists,
                    message: format!("{} already exists", to.display()),
                };
            }
            fs::rename(&from, &to).map(|()| Reply::Done)
        }
        Request::Delete { path, recursive } => {
            let p = resolve(home, &path);
            if p.parent().is_none() || p == home {
                return invalid("refusing to delete the root or the home directory");
            }
            match fs::symlink_metadata(&p) {
                // A link is removed, never followed.
                Ok(m) if m.is_dir() => {
                    if recursive {
                        fs::remove_dir_all(&p)
                    } else {
                        fs::remove_dir(&p)
                    }
                }
                Ok(_) => fs::remove_file(&p),
                Err(e) => Err(e),
            }
            .map(|()| Reply::Done)
        }
    };
    result.unwrap_or_else(err)
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
        meta.permissions().mode()
    };
    #[cfg(not(unix))]
    let mode = 0;
    Ok(Entry {
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
    let mut f = fs::File::open(p)?;
    if f.metadata()?.is_dir() {
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
    fn it_refuses_what_would_destroy_more_than_asked() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::write(h.join("a"), b"a").unwrap();
        fs::write(h.join("b"), b"b").unwrap();
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
            ],
        );
        assert!(matches!(
            replies[0],
            Reply::Err {
                kind: ErrKind::AlreadyExists,
                ..
            }
        ));
        assert!(matches!(
            replies[1],
            Reply::Err {
                kind: ErrKind::Invalid,
                ..
            }
        ));
        assert!(matches!(
            replies[2],
            Reply::Err {
                kind: ErrKind::Invalid,
                ..
            }
        ));
        assert!(matches!(replies[3], Reply::Err { .. }));
        assert_eq!(fs::read(h.join("b")).unwrap(), b"b");
        assert!(h.exists());
    }
}
