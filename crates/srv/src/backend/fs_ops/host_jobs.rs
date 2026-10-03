// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copy and move between this computer and an SSH host, or within one host
//! (spec §6.3 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md):
//! the same `fs.op` as a local copy, with the same progress, cancel and
//! conflict questions (`jobs::Jobs::start_host`), the host's side carried by
//! its helper (`remote::files`).
//!
//! - A file goes to a temp file beside its destination, in pieces (ranged
//!   reads, appends), and is renamed into place only when complete, never
//!   over anything the user didn't choose to replace.
//! - A folder onto a folder merges; anything else already there is asked
//!   about (replace, skip, keep both).
//! - Links aren't copied between two machines: where one points means
//!   something only on its own side.
//! - A move removes each source only once it landed; within one host it is a
//!   rename on that host.
//! - Nothing in a protected place is changed: this computer's rules
//!   (`ProtectedPaths`), a host's system folders, root and home folder
//!   (`remote::protected`).

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::runtime::Handle;

use super::jobs::{EntrySide, HostJob, HostStop, JOBS};
use super::platform::{self, display_path};
use super::ProtectedPaths;
use crate::backend::remote::files::{RemoteError, RemoteFiles};
use crate::backend::rpc_types::{FsOpChoice, FsOpKind, FsOpStartResult};

/// One side of a transfer.
#[derive(Clone)]
pub enum End {
    /// This computer.
    Local,
    /// An SSH host, through its helper; `home` in tidy form.
    Host {
        files: Arc<RemoteFiles>,
        home: String,
    },
}

/// What a path is, on either side.
#[derive(Debug, Clone, Copy)]
struct Info {
    is_dir: bool,
    is_link: bool,
    /// A regular file (not a device, socket or pipe).
    is_file: bool,
    size: u64,
    mtime: Option<u64>,
    /// Unix permission bits; 0 where there are none.
    mode: u32,
}

impl Info {
    fn side(&self) -> EntrySide {
        EntrySide {
            is_dir: self.is_dir,
            size: (!self.is_dir).then_some(self.size),
            mtime: self.mtime,
        }
    }
}

/// A rename that found its target taken.
enum RenameError {
    Taken,
    Other(String),
}

fn host_error(e: RemoteError) -> String {
    e.message
}

impl End {
    pub fn host(files: Arc<RemoteFiles>) -> Self {
        let home = super::remote::resolve("/", &files.home);
        End::Host { files, home }
    }

    /// Both are the same host's helper.
    fn same_host(&self, other: &End) -> bool {
        match (self, other) {
            (End::Host { files: a, .. }, End::Host { files: b, .. }) => a.conn == b.conn,
            _ => false,
        }
    }

    fn join(&self, dir: &str, name: &str) -> String {
        match self {
            End::Local => display_path(&Path::new(dir).join(name)),
            End::Host { .. } => posix_join(dir, name),
        }
    }

    fn parent(&self, path: &str) -> String {
        match self {
            End::Local => Path::new(path)
                .parent()
                .map(display_path)
                .unwrap_or_default(),
            End::Host { .. } => posix_parent(path),
        }
    }

    /// Why `path` may not be changed, if it may not.
    fn refusal(&self, path: &str) -> Option<String> {
        match self {
            End::Local => ProtectedPaths::current().check(Path::new(path)).err(),
            End::Host { files, home } => super::remote::protected(home, path).then(|| {
                format!("AgentMux doesn't change {path} on {}: it's a system folder or your home folder.", files.conn)
            }),
        }
    }

    fn stat(&self, rt: &Handle, path: &str) -> Result<Option<Info>, String> {
        match self {
            End::Local => match std::fs::symlink_metadata(path) {
                Ok(m) => Ok(Some(Info {
                    is_dir: m.is_dir(),
                    is_link: m.file_type().is_symlink(),
                    is_file: m.is_file(),
                    mode: local_mode(&m),
                    size: m.len(),
                    mtime: m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64),
                })),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.to_string()),
            },
            End::Host { files, .. } => match rt.block_on(files.stat(path)) {
                Ok(e) => Ok(Some(Info {
                    is_dir: e.kind == agentmux_remote::fsproto::Kind::Dir && !e.symlink,
                    is_link: e.symlink,
                    is_file: e.kind == agentmux_remote::fsproto::Kind::File && !e.symlink,
                    mode: e.mode,
                    size: e.size,
                    mtime: (e.mtime_ms > 0).then_some(e.mtime_ms as u64),
                })),
                Err(e) if e.is_not_found() => Ok(None),
                Err(e) => Err(host_error(e)),
            },
        }
    }

    fn list(&self, rt: &Handle, dir: &str) -> Result<Vec<String>, String> {
        match self {
            End::Local => {
                let mut names: Vec<String> = std::fs::read_dir(dir)
                    .map_err(|e| e.to_string())?
                    .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                    .collect();
                names.sort();
                Ok(names)
            }
            End::Host { files, .. } => Ok(rt
                .block_on(files.list(dir))
                .map_err(host_error)?
                .into_iter()
                .map(|e| e.name)
                .collect()),
        }
    }

    fn mkdir(&self, rt: &Handle, path: &str) -> Result<(), String> {
        match self {
            End::Local => std::fs::create_dir(path).map_err(|e| e.to_string()),
            End::Host { files, .. } => rt.block_on(files.mkdir(path, false)).map_err(host_error),
        }
    }

    /// Up to `len` bytes of a file from `offset`, and whether it ends there.
    fn read(
        &self,
        rt: &Handle,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<(Vec<u8>, bool), String> {
        match self {
            End::Local => {
                let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
                f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                let mut data = Vec::with_capacity(len);
                f.take(len as u64)
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
                let eof = data.len() < len;
                Ok((data, eof))
            }
            End::Host { files, .. } => rt
                .block_on(files.read_range(path, offset, len as u32))
                .map_err(host_error),
        }
    }

    /// A new, empty temp file in `dir`, for a piece-by-piece copy.
    fn create_temp(&self, rt: &Handle, dir: &str) -> Result<String, String> {
        let name = format!(".agentmux-part-{}", uuid::Uuid::new_v4().simple());
        let path = self.join(dir, &name);
        match self {
            End::Local => {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
            }
            End::Host { files, .. } => rt
                .block_on(files.write(&path, Vec::new()))
                .map_err(host_error)?,
        }
        Ok(path)
    }

    fn append(&self, rt: &Handle, path: &str, data: Vec<u8>) -> Result<(), String> {
        match self {
            End::Local => std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(&data))
                .map_err(|e| e.to_string()),
            End::Host { files, .. } => rt.block_on(files.append(path, data)).map_err(host_error),
        }
    }

    /// Rename, never over anything.
    fn rename_no_replace(&self, rt: &Handle, from: &str, to: &str) -> Result<(), RenameError> {
        match self {
            End::Local => {
                platform::rename_no_replace(Path::new(from), Path::new(to)).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        RenameError::Taken
                    } else {
                        RenameError::Other(e.to_string())
                    }
                })
            }
            End::Host { files, .. } => rt.block_on(files.rename(from, to)).map_err(|e| {
                if e.kind == Some(agentmux_remote::fsproto::ErrKind::AlreadyExists) {
                    RenameError::Taken
                } else {
                    RenameError::Other(e.message)
                }
            }),
        }
    }

    /// Rename the file `from` over the file `to` in one step: what is there
    /// goes only once the new one is in place.
    fn replace(&self, rt: &Handle, from: &str, to: &str) -> Result<(), String> {
        match self {
            End::Local => std::fs::rename(from, to).map_err(|e| e.to_string()),
            End::Host { files, .. } => rt.block_on(files.replace(from, to)).map_err(host_error),
        }
    }

    /// Give a file the source's permission bits and modification time.
    fn set_meta(
        &self,
        rt: &Handle,
        path: &str,
        mode: u32,
        mtime: Option<u64>,
    ) -> Result<(), String> {
        let mtime_ms = mtime.unwrap_or(0) as i64;
        match self {
            End::Local => {
                if mtime_ms > 0 {
                    let t =
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(mtime_ms as u64);
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(path)
                        .and_then(|f| f.set_modified(t))
                        .map_err(|e| e.to_string())?;
                }
                #[cfg(unix)]
                if mode != 0 {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
                        .map_err(|e| e.to_string())?;
                }
                #[cfg(not(unix))]
                let _ = mode;
                Ok(())
            }
            End::Host { files, .. } => rt
                .block_on(files.set_meta(path, mode, mtime_ms))
                .map_err(host_error),
        }
    }

    /// Flush a file's contents to disk.
    fn sync(&self, rt: &Handle, path: &str) -> Result<(), String> {
        match self {
            End::Local => std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string()),
            End::Host { files, .. } => rt.block_on(files.sync(path)).map_err(host_error),
        }
    }

    /// Remove a file, a link, or an empty folder. A read-only file goes too
    /// (Windows refuses to remove one otherwise).
    fn remove(&self, rt: &Handle, path: &str) -> Result<(), String> {
        match self {
            End::Local => {
                let p = Path::new(path);
                match std::fs::symlink_metadata(p) {
                    Ok(m) if m.is_file() => platform::remove_file_force(p),
                    _ => platform::remove_entry_no_follow(p),
                }
                .map_err(|e| e.to_string())
            }
            End::Host { files, .. } => rt.block_on(files.delete(path, false)).map_err(host_error),
        }
    }

    /// The last part of `path` as this side spells paths: on a host `/` only
    /// (a `\` there is part of a name); here, either separator.
    fn name(&self, path: &str) -> String {
        match self {
            End::Local => file_name(path),
            End::Host { .. } => path
                .rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or(path)
                .to_string(),
        }
    }

    /// The display form of `path` on this side, naming the host.
    fn show(&self, path: &str) -> String {
        match self {
            End::Local => path.to_string(),
            End::Host { files, .. } => format!("{}:{path}", files.conn),
        }
    }

    /// `path` as this side spells it: a host's resolved under its home.
    fn tidy(&self, path: &str) -> String {
        match self {
            End::Local => display_path(&PathBuf::from(path)),
            End::Host { home, .. } => super::remote::resolve(home, path),
        }
    }
}

/// `name` in `dir`, a host's POSIX path.
fn posix_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The folder a host's POSIX path is in; `/` at the top.
fn posix_parent(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

/// A local entry's Unix permission bits (0 elsewhere).
fn local_mode(m: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

/// A local source file, opened once: never through a link swapped in after
/// it was checked, never stuck opening a pipe; whatever was opened must be a
/// regular file.
fn open_source(path: &str) -> Result<std::fs::File, String> {
    #[cfg(unix)]
    let f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| e.to_string())?
    };
    #[cfg(not(unix))]
    let f = platform::open_no_follow(Path::new(path)).map_err(|e| e.to_string())?;
    match f.metadata() {
        Ok(m) if m.is_file() => Ok(f),
        Ok(_) => Err(format!(
            "“{}” changed while it was being copied: not a regular file now.",
            file_name(path)
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// Up to `len` bytes from an open file, and whether it ends there.
fn read_open(f: &mut std::fs::File, len: usize) -> Result<(Vec<u8>, bool), String> {
    let mut data = Vec::with_capacity(len);
    f.take(len as u64)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let eof = data.len() < len;
    Ok((data, eof))
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// `name (n).ext` for the first free n.
fn keep_both_name(end: &End, rt: &Handle, dir: &str, name: &str) -> Result<String, String> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..10_000 {
        let candidate = end.join(dir, &format!("{stem} ({n}){ext}"));
        if end.stat(rt, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(format!("No free name for a copy of “{name}”."))
}

/// What became of one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Done,
    NotDone,
    Stopped,
}

struct Transfer<'r> {
    rt: &'r Handle,
    kind: FsOpKind,
    src: End,
    dest: End,
    /// Each measured subtree's (items, bytes), by source path.
    measured: std::cell::RefCell<std::collections::HashMap<String, (u64, u64)>>,
}

impl Transfer<'_> {
    /// Items and file bytes under `path` on the source.
    /// Items and file bytes under `path` on the source, each subtree's
    /// totals kept for [`Self::pass_over`]. Stops early when the op is
    /// canceled: every entry is a round trip to a host.
    fn measure(&self, job: &HostJob<'_, '_>, path: &str) -> (u64, u64) {
        if job.is_canceled() {
            return (0, 0);
        }
        let Ok(Some(info)) = self.src.stat(self.rt, path) else {
            return (0, 0);
        };
        let mut total = (1u64, 0u64);
        if info.is_dir {
            for name in self.src.list(self.rt, path).unwrap_or_default() {
                let (i, b) = self.measure(job, &self.src.join(path, &name));
                total.0 += i;
                total.1 += b;
            }
        } else if !info.is_link {
            total.1 = info.size;
        }
        self.measured.borrow_mut().insert(path.to_string(), total);
        total
    }

    /// Count `path`'s subtree as handled without copying it (skipped,
    /// failed, or moved by a rename), from what [`Self::measure`] found.
    fn pass_over(&self, job: &mut HostJob<'_, '_>, path: &str) {
        let (items, bytes) = self.measured.borrow().get(path).copied().unwrap_or((1, 0));
        job.advance(items, bytes);
    }

    fn entry(&self, job: &mut HostJob<'_, '_>, src: &str, dest: &str) -> Outcome {
        if job.is_canceled() {
            return Outcome::Stopped;
        }
        let shown = self.src.show(src);
        job.set_current(&shown);
        let info = match self.src.stat(self.rt, src) {
            Ok(Some(i)) => i,
            Ok(None) => {
                job.fail(
                    &shown,
                    format!("“{}” isn't there anymore.", self.src.name(src)),
                );
                return Outcome::NotDone;
            }
            Err(e) => {
                job.fail(&shown, e);
                return Outcome::NotDone;
            }
        };
        // A link moves within one host as itself (a rename there); copied, or
        // between two machines, it isn't: where it points means something
        // only on its own side, and the helper makes no links.
        let one_host = self.src.same_host(&self.dest);
        if info.is_link && !(one_host && self.kind == FsOpKind::Move) {
            let why = if one_host {
                format!(
                    "“{}” is a link: copying links on a host isn't available yet (moving one is).",
                    self.src.name(src)
                )
            } else {
                format!(
                    "“{}” is a link: links aren't copied between two machines.",
                    self.src.name(src)
                )
            };
            job.fail(&shown, why);
            job.advance(1, 0);
            return Outcome::NotDone;
        }
        if !info.is_dir && !info.is_link && !info.is_file {
            job.fail(
                &shown,
                format!(
                    "“{}” isn't a regular file (a device, socket or pipe): not copied.",
                    self.src.name(src)
                ),
            );
            job.advance(1, 0);
            return Outcome::NotDone;
        }
        if let Some(why) = self.dest.refusal(dest) {
            job.fail(&shown, why);
            self.pass_over(job, src);
            return Outcome::NotDone;
        }
        if self.kind == FsOpKind::Move {
            if let Some(why) = self.src.refusal(src) {
                job.fail(&shown, why);
                self.pass_over(job, src);
                return Outcome::NotDone;
            }
        }
        let existing = match self.dest.stat(self.rt, dest) {
            Ok(e) => e,
            Err(e) => {
                job.fail(&shown, e);
                self.pass_over(job, src);
                return Outcome::NotDone;
            }
        };
        let mut dest = dest.to_string();
        let mut replace = false;
        if let Some(there) = existing {
            if info.is_dir && there.is_dir {
                return self.merge(job, src, &dest);
            }
            match job.ask(&shown, info.side(), &self.dest.show(&dest), there.side()) {
                None => return Outcome::Stopped,
                Some(FsOpChoice::Skip) => {
                    self.pass_over(job, src);
                    return Outcome::NotDone;
                }
                Some(FsOpChoice::KeepBoth) => {
                    let dir = self.dest.parent(&dest);
                    match keep_both_name(&self.dest, self.rt, &dir, &self.dest.name(&dest)) {
                        Ok(d) => dest = d,
                        Err(e) => {
                            job.fail(&shown, e);
                            self.pass_over(job, src);
                            return Outcome::NotDone;
                        }
                    }
                }
                Some(FsOpChoice::Replace) => {
                    if info.is_dir || there.is_dir {
                        job.fail(
                            &shown,
                            format!(
                                "“{}” and what's there aren't both files: not replaced.",
                                self.src.name(src)
                            ),
                        );
                        self.pass_over(job, src);
                        return Outcome::NotDone;
                    }
                    replace = true;
                }
            }
        }
        // Within one host, a move is a rename there. A link only ever moves
        // this way (never read through): to replace, what is there goes first.
        if info.is_link && replace {
            // Only a move within one host gets here: in one step there.
            return match self.src.replace(self.rt, src, &dest) {
                Ok(()) => {
                    self.pass_over(job, src);
                    Outcome::Done
                }
                Err(e) => {
                    job.fail(&shown, e);
                    job.advance(1, 0);
                    Outcome::NotDone
                }
            };
        }
        if self.kind == FsOpKind::Move && !replace && one_host {
            match self.src.rename_no_replace(self.rt, src, &dest) {
                Ok(()) => {
                    self.pass_over(job, src);
                    return Outcome::Done;
                }
                Err(RenameError::Taken) => {
                    job.fail(
                        &shown,
                        format!(
                            "Something named “{}” appeared there.",
                            self.dest.name(&dest)
                        ),
                    );
                    self.pass_over(job, src);
                    return Outcome::NotDone;
                }
                // A link can't be copied instead.
                Err(RenameError::Other(e)) if info.is_link => {
                    job.fail(&shown, e);
                    job.advance(1, 0);
                    return Outcome::NotDone;
                }
                Err(RenameError::Other(_)) => {}
            }
        }
        if info.is_dir {
            if let Err(e) = self.dest.mkdir(self.rt, &dest) {
                job.fail(&shown, e);
                self.pass_over(job, src);
                return Outcome::NotDone;
            }
            job.advance(1, 0);
            let out = self.children(job, src, &dest);
            if out == Outcome::Done && self.kind == FsOpKind::Move {
                if let Err(e) = self.src.remove(self.rt, src) {
                    job.fail(&shown, e);
                    return Outcome::NotDone;
                }
            }
            return out;
        }
        self.file(job, src, &dest, &shown, info, replace)
    }

    fn merge(&self, job: &mut HostJob<'_, '_>, src: &str, dest: &str) -> Outcome {
        job.advance(1, 0);
        let out = self.children(job, src, dest);
        if out == Outcome::Done && self.kind == FsOpKind::Move {
            if let Err(e) = self.src.remove(self.rt, src) {
                job.fail(&self.src.show(src), e);
                return Outcome::NotDone;
            }
        }
        out
    }

    fn children(&self, job: &mut HostJob<'_, '_>, src: &str, dest: &str) -> Outcome {
        let names = match self.src.list(self.rt, src) {
            Ok(n) => n,
            Err(e) => {
                job.fail(&self.src.show(src), e);
                return Outcome::NotDone;
            }
        };
        let mut all = Outcome::Done;
        for name in names {
            match self.entry(
                job,
                &self.src.join(src, &name),
                &self.dest.join(dest, &name),
            ) {
                Outcome::Stopped => return Outcome::Stopped,
                Outcome::NotDone => all = Outcome::NotDone,
                Outcome::Done => {}
            }
        }
        all
    }

    fn file(
        &self,
        job: &mut HostJob<'_, '_>,
        src: &str,
        dest: &str,
        shown: &str,
        info: Info,
        replace: bool,
    ) -> Outcome {
        let size = info.size;
        let dir = self.dest.parent(dest);
        let temp = match self.dest.create_temp(self.rt, &dir) {
            Ok(t) => t,
            Err(e) => {
                job.fail(shown, e);
                job.advance(1, size);
                return Outcome::NotDone;
            }
        };
        let discard = |t: &Transfer<'_>| {
            let _ = t.dest.remove(t.rt, &temp);
        };
        // A local source: opened once, checked, then read from that handle.
        let mut local_src = match &self.src {
            End::Local => match open_source(src) {
                Ok(f) => Some(f),
                Err(e) => {
                    discard(self);
                    job.fail(shown, e);
                    job.advance(1, size);
                    return Outcome::NotDone;
                }
            },
            End::Host { .. } => None,
        };
        // Pieces: the job's chunk, within what one message may carry.
        let chunk = job.chunk_size().clamp(64 * 1024, 4 << 20);
        let mut offset = 0u64;
        loop {
            if job.is_canceled() {
                discard(self);
                return Outcome::Stopped;
            }
            let read = match local_src.as_mut() {
                Some(f) => read_open(f, chunk),
                None => self.src.read(self.rt, src, offset, chunk),
            };
            let (data, eof) = match read {
                Ok(r) => r,
                Err(e) => {
                    discard(self);
                    job.fail(shown, e);
                    job.advance(1, size.saturating_sub(offset));
                    return Outcome::NotDone;
                }
            };
            let n = data.len() as u64;
            if n > 0 {
                if let Err(e) = self.dest.append(self.rt, &temp, data) {
                    discard(self);
                    job.fail(shown, e);
                    job.advance(1, size.saturating_sub(offset));
                    return Outcome::NotDone;
                }
                offset += n;
                job.advance(0, n);
            }
            if eof || n == 0 {
                break;
            }
        }
        // The source's permissions and time, on the temp file before it goes
        // in: never a half-kept file in place, and a moved program still runs.
        // Best effort, as for a local copy: a disk that refuses (FAT, some
        // network shares) still gets the file.
        if let Err(e) = self.dest.set_meta(self.rt, &temp, info.mode, info.mtime) {
            tracing::debug!(path = %shown, error = %e, "fs.op: a host copy's permissions or time weren't kept");
        }
        // A move deletes the only other copy next: on disk first.
        if self.kind == FsOpKind::Move {
            if let Err(e) = self.dest.sync(self.rt, &temp) {
                discard(self);
                job.fail(shown, format!("Couldn't make sure it was written: {e}"));
                job.advance(1, 0);
                return Outcome::NotDone;
            }
        }
        // Replacing: in one step, so what is there goes only once the copy is.
        let placed = if replace {
            self.dest
                .replace(self.rt, &temp, dest)
                .map_err(RenameError::Other)
        } else {
            self.dest.rename_no_replace(self.rt, &temp, dest)
        };
        match placed {
            Ok(()) => {}
            Err(RenameError::Taken) => {
                discard(self);
                job.fail(
                    shown,
                    format!(
                        "Something named “{}” appeared there meanwhile; nothing was replaced.",
                        self.dest.name(dest)
                    ),
                );
                job.advance(1, 0);
                return Outcome::NotDone;
            }
            Err(RenameError::Other(e)) => {
                discard(self);
                job.fail(shown, e);
                job.advance(1, 0);
                return Outcome::NotDone;
            }
        }
        job.advance(1, 0);
        if self.kind == FsOpKind::Move {
            if let Err(e) = self.src.remove(self.rt, src) {
                job.fail(
                    shown,
                    format!("Copied, but the original couldn't be removed: {e}"),
                );
                return Outcome::NotDone;
            }
        }
        Outcome::Done
    }
}

/// One source to transfer, as planned.
struct Item {
    src: String,
    /// Copied into the folder it is already in: kept as `name (n)`.
    duplicate: bool,
}

fn under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
        || (dir == "/" && path.starts_with('/') && path != "/")
}

/// The local plan's rules (`jobs::plan`) where source and destination are
/// one host's folders: a folder never into itself or a folder of its own; a
/// move into the folder it is in is nothing to do, a copy there is a
/// duplicate; and no destination lands on a folder holding another source.
/// Between two machines none of this can happen.
/// `sources` are `(as given, resolved)` pairs: the resolved one has its
/// parent's links resolved (`Realpath`), its own name kept, so a link to an
/// ancestor isn't taken for the ancestor; `dest_dir` is resolved too.
fn plan_on_one_host(
    kind: FsOpKind,
    src: &End,
    sources: &[(String, String)],
    dest: &End,
    dest_dir: &str,
) -> Result<Vec<Item>, String> {
    if !src.same_host(dest) {
        return Ok(sources
            .iter()
            .map(|(s, _)| Item {
                src: s.clone(),
                duplicate: false,
            })
            .collect());
    }
    let verb = if kind == FsOpKind::Move {
        "move"
    } else {
        "copy"
    };
    let mut items = Vec::new();
    let mut keys = Vec::new();
    for (s, key) in sources {
        if dest_dir == key || under(dest_dir, key) {
            return Err(format!("Can't {verb} a folder into itself."));
        }
        let in_dest_already = src.parent(key) == dest_dir;
        if in_dest_already && kind == FsOpKind::Move {
            continue;
        }
        items.push(Item {
            src: s.clone(),
            duplicate: in_dest_already,
        });
        keys.push(key.clone());
    }
    // E.g. copying `/a/d/d` into `/a` would merge into `/a/d`, which holds
    // the source itself.
    let landing: std::collections::HashSet<String> = items
        .iter()
        .filter(|i| !i.duplicate)
        .map(|i| src.name(&i.src))
        .collect();
    for (item, key) in items.iter().zip(&keys) {
        let rest = if dest_dir == "/" {
            key.strip_prefix('/')
        } else {
            key.strip_prefix(&format!("{dest_dir}/"))
        };
        let Some(rest) = rest else { continue };
        let mut parts = rest.split('/').filter(|p| !p.is_empty());
        if let (Some(first), Some(_)) = (parts.next(), parts.next()) {
            if landing.contains(first) {
                return Err(format!(
                    "Can't {verb} there: “{first}” would land on the folder that holds “{}”.",
                    src.name(&item.src)
                ));
            }
        }
    }
    Ok(items)
}

/// `path`'s entry on `end`, awaited (before the op's thread exists).
async fn stat_now(end: &End, path: &str) -> Result<Option<()>, String> {
    match end {
        End::Local => match std::fs::symlink_metadata(path) {
            Ok(_) => Ok(Some(())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        },
        End::Host { files, .. } => match files.stat(path).await {
            Ok(_) => Ok(Some(())),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(host_error(e)),
        },
    }
}

/// `path` with its links resolved, awaited; as given when it can't be.
async fn realpath_now(end: &End, path: &str) -> String {
    match end {
        End::Local => std::fs::canonicalize(path)
            .map(|p| display_path(&p))
            .unwrap_or_else(|_| path.to_string()),
        End::Host { files, .. } => files
            .realpath(path)
            .await
            .unwrap_or_else(|_| path.to_string()),
    }
}

/// Start a copy or move of `sources` (on `src`) into `dest_dir` (on `dest`)
/// as an `fs.op` (`emit` carries its events). At least one side is a host.
pub async fn start(
    kind: FsOpKind,
    src: End,
    sources: Vec<String>,
    dest: End,
    dest_dir: String,
    emit: super::jobs::Emit,
) -> Result<FsOpStartResult, String> {
    let rt = Handle::current();
    // Where things really are: the destination folder and each source's
    // parent with their links resolved (a source keeps its own name, so a
    // link is moved as a link). Every check below, and the transfer itself,
    // use these, so a link in a parent (`/tmp/x` to `/home`) can't spell its
    // way past a protected place.
    let dest_dir = realpath_now(&dest, &dest.tidy(&dest_dir)).await;
    let mut resolved = Vec::with_capacity(sources.len());
    for s in &sources {
        let s = src.tidy(s);
        let parent = realpath_now(&src, &src.parent(&s)).await;
        resolved.push(src.join(&parent, &src.name(&s)));
    }
    let sources = resolved;
    // Checked before answering, as a local copy's plan is (awaited here: only
    // the op's own thread blocks on the runtime).
    let is_dir = match &dest {
        End::Local => std::fs::metadata(&dest_dir)
            .map(|m| m.is_dir())
            .map_err(|e| e.kind()),
        End::Host { files, .. } => files
            .stat(&dest_dir)
            .await
            .map(|e| e.kind == agentmux_remote::fsproto::Kind::Dir)
            .map_err(|e| {
                if e.is_not_found() {
                    std::io::ErrorKind::NotFound
                } else {
                    std::io::ErrorKind::Other
                }
            }),
    };
    match is_dir {
        Ok(true) => {}
        Ok(false) => return Err("The destination isn't a folder.".to_string()),
        Err(std::io::ErrorKind::NotFound) => {
            return Err("The destination folder isn't there anymore.".to_string())
        }
        Err(_) => return Err("Couldn't reach the destination folder.".to_string()),
    }
    if sources.is_empty() {
        return Err("Nothing to copy.".to_string());
    }
    // Each source there, and nothing protected, before an op id is given: a
    // cut is used up only once the move has been accepted.
    for s in &sources {
        match stat_now(&src, s).await? {
            Some(_) => {}
            None => return Err(format!("“{}” isn't there anymore.", src.name(s))),
        }
        if let Some(why) = dest.refusal(&dest.join(&dest_dir, &src.name(s))) {
            return Err(why);
        }
        if kind == FsOpKind::Move {
            if let Some(why) = src.refusal(s) {
                return Err(why);
            }
        }
    }
    let keyed: Vec<(String, String)> = sources.iter().map(|s| (s.clone(), s.clone())).collect();
    let items = plan_on_one_host(kind, &src, &keyed, &dest, &dest_dir)?;
    let shown_dest = dest.show(&dest_dir);
    let requested = sources.len();
    JOBS.start_host(
        kind,
        requested,
        shown_dest,
        emit,
        Box::new(move |job| {
            let t = Transfer {
                rt: &rt,
                kind,
                src,
                dest,
                measured: Default::default(),
            };
            let (mut total_items, mut total_bytes) = (0, 0);
            for item in &items {
                let (i, b) = t.measure(job, &item.src);
                total_items += i;
                total_bytes += b;
            }
            if job.is_canceled() {
                return Err(HostStop::Canceled);
            }
            job.add_total(total_items, total_bytes);
            for item in &items {
                let name = t.src.name(&item.src);
                let target = if item.duplicate {
                    // Copied into the folder it is in: always beside itself,
                    // as `name (n)`, never a question about itself.
                    match keep_both_name(&t.dest, &rt, &dest_dir, &name) {
                        Ok(d) => d,
                        Err(e) => {
                            job.fail(&t.src.show(&item.src), e);
                            t.pass_over(job, &item.src);
                            continue;
                        }
                    }
                } else {
                    t.dest.join(&dest_dir, &name)
                };
                if t.entry(job, &item.src, &target) == Outcome::Stopped {
                    return Err(HostStop::Canceled);
                }
            }
            Ok(())
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{FsOpEvent, FsOpEventState};
    use std::sync::Mutex;

    fn events() -> (super::super::jobs::Emit, Arc<Mutex<Vec<FsOpEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (
            Arc::new(move |e: &FsOpEvent| s.lock().unwrap().push(e.clone())),
            seen,
        )
    }

    async fn finished(seen: &Arc<Mutex<Vec<FsOpEvent>>>) -> FsOpEvent {
        for _ in 0..500 {
            if let Some(e) = seen
                .lock()
                .unwrap()
                .iter()
                .find(|e| {
                    matches!(
                        e.state,
                        FsOpEventState::Done | FsOpEventState::Failed | FsOpEventState::Canceled
                    )
                })
                .cloned()
            {
                return e;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the op never finished");
    }

    async fn host(home: &Path) -> End {
        let (r, w) = crate::backend::remote::files::testing::helper(home.to_path_buf());
        End::host(
            RemoteFiles::over("testhost", r, w, Box::new(()))
                .await
                .unwrap(),
        )
    }

    /// Up to a host and back down, folders and all, in pieces bigger than
    /// one read; links stay behind with a reason.
    // Linux: a host's paths are POSIX, so the test home's must be.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn files_and_folders_go_up_to_a_host_and_back_down() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..(5 << 20) + 17).map(|i| (i % 251) as u8).collect();
        std::fs::create_dir_all(local.path().join("proj/src")).unwrap();
        std::fs::write(local.path().join("proj/src/big.bin"), &big).unwrap();
        std::fs::write(local.path().join("proj/readme.md"), b"hi").unwrap();
        let h = host(remote.path()).await;

        let (emit, seen) = events();
        let src = display_path(&local.path().join("proj"));
        let dest_dir = display_path(remote.path());
        start(
            FsOpKind::Copy,
            End::Local,
            vec![src],
            h.clone(),
            dest_dir,
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(end.done_bytes, end.total_bytes);
        assert_eq!(
            std::fs::read(remote.path().join("proj/src/big.bin")).unwrap(),
            big
        );
        assert_eq!(
            std::fs::read(remote.path().join("proj/readme.md")).unwrap(),
            b"hi"
        );
        // No temp file left on the host.
        assert!(std::fs::read_dir(remote.path().join("proj/src"))
            .unwrap()
            .all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".agentmux-part")));

        // Back down, as a move: the host's copy goes once it landed.
        let down = tempfile::tempdir().unwrap();
        let (emit, seen) = events();
        let src = display_path(&remote.path().join("proj"));
        start(
            FsOpKind::Move,
            h,
            vec![src],
            End::Local,
            display_path(down.path()),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(
            std::fs::read(down.path().join("proj/src/big.bin")).unwrap(),
            big
        );
        assert!(!remote.path().join("proj").exists());
    }

    /// Something already at the destination is asked about; Skip leaves it,
    /// and Keep both puts the copy beside it.
    // Linux: a host's paths are POSIX, so the test home's must be.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_existing_file_is_asked_about_never_overwritten_silently() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"new").unwrap();
        std::fs::write(remote.path().join("a.txt"), b"old").unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        let op = start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("a.txt"))],
            h,
            display_path(remote.path()),
            emit,
        )
        .await
        .unwrap();
        // Wait for the question, then keep both.
        for _ in 0..500 {
            if seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.state == FsOpEventState::Conflict)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let asked = seen
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.state == FsOpEventState::Conflict)
            .cloned()
            .unwrap();
        let c = asked.conflict.unwrap();
        assert_eq!((c.source_size, c.dest_size), (Some(3), Some(3)));
        assert!(c.dest.starts_with("testhost:"), "{}", c.dest);
        JOBS.resolve(&op.op_id, FsOpChoice::KeepBoth, false)
            .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(std::fs::read(remote.path().join("a.txt")).unwrap(), b"old");
        assert_eq!(
            std::fs::read(remote.path().join("a (2).txt")).unwrap(),
            b"new"
        );
    }

    /// Within one host a move is a rename there; nothing comes through here.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_move_within_one_host_is_a_rename_there() {
        let remote = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(remote.path().join("a")).unwrap();
        std::fs::create_dir_all(remote.path().join("b")).unwrap();
        std::fs::write(remote.path().join("a/x.txt"), b"x").unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        start(
            FsOpKind::Move,
            h.clone(),
            vec![display_path(&remote.path().join("a/x.txt"))],
            h,
            display_path(&remote.path().join("b")),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(std::fs::read(remote.path().join("b/x.txt")).unwrap(), b"x");
        assert!(!remote.path().join("a/x.txt").exists());
    }

    /// Canceled mid-file: nothing half-written is left on the host.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_canceled_upload_leaves_nothing_behind() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("huge.bin"), vec![1u8; 16 << 20]).unwrap();
        // A slow link: each piece takes a while, so the cancel lands halfway.
        let (r, w) = crate::backend::remote::files::testing::slow_helper(
            remote.path().to_path_buf(),
            std::time::Duration::from_millis(20),
        );
        let h = End::host(
            RemoteFiles::over("testhost", r, w, Box::new(()))
                .await
                .unwrap(),
        );
        let (emit, seen) = events();
        let op = start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("huge.bin"))],
            h,
            display_path(remote.path()),
            emit,
        )
        .await
        .unwrap();
        for _ in 0..500 {
            if seen.lock().unwrap().iter().any(|e| e.done_bytes > 0) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        JOBS.cancel(&op.op_id);
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Canceled, "{end:?}");
        let left: Vec<String> = std::fs::read_dir(remote.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    /// A host's system folders are never written, whatever is asked.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_protected_folder_on_the_host_is_refused() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"a").unwrap();
        let h = host(remote.path()).await;
        // Refused before an op exists.
        let err = start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("a.txt"))],
            h,
            "/etc".into(),
            events().0,
        )
        .await
        .unwrap_err();
        assert!(err.contains("doesn't change"), "{err}");
        assert!(!std::path::Path::new("/etc/a.txt").exists());
    }

    /// Within one host: never a folder into itself, a copy into its own
    /// folder is a duplicate, and nothing lands on a folder holding a source.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn within_one_host_the_local_plans_rules_hold() {
        let remote = tempfile::tempdir().unwrap();
        let r = display_path(remote.path());
        std::fs::create_dir_all(remote.path().join("d/d/inner")).unwrap();
        std::fs::write(remote.path().join("d/a.txt"), b"a").unwrap();
        let h = host(remote.path()).await;
        let into_itself = start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/d")],
            h.clone(),
            format!("{r}/d/d"),
            events().0,
        )
        .await;
        assert!(into_itself.unwrap_err().contains("into itself"));
        let landing = start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/d/d")],
            h.clone(),
            r.clone(),
            events().0,
        )
        .await;
        assert!(landing.unwrap_err().contains("would land on the folder"));

        // A copy beside itself: `a (2).txt`, nothing asked.
        let (emit, seen) = events();
        start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/d/a.txt")],
            h,
            format!("{r}/d"),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == FsOpEventState::Conflict));
        assert_eq!(
            std::fs::read(remote.path().join("d/a (2).txt")).unwrap(),
            b"a"
        );
        assert_eq!(std::fs::read(remote.path().join("d/a.txt")).unwrap(), b"a");
    }

    /// Within one host a link moves as itself; copied, it is reported.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_link_moves_within_one_host_and_is_not_copied() {
        let remote = tempfile::tempdir().unwrap();
        let r = display_path(remote.path());
        std::fs::create_dir_all(remote.path().join("to")).unwrap();
        std::fs::write(remote.path().join("target.txt"), b"t").unwrap();
        std::os::unix::fs::symlink("target.txt", remote.path().join("l1")).unwrap();
        std::os::unix::fs::symlink("target.txt", remote.path().join("l2")).unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        start(
            FsOpKind::Move,
            h.clone(),
            vec![format!("{r}/l1")],
            h.clone(),
            format!("{r}/to"),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert!(end.failures.is_none(), "{end:?}");
        let moved = remote.path().join("to/l1");
        assert!(std::fs::symlink_metadata(&moved)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_link(&moved).unwrap(),
            std::path::Path::new("target.txt")
        );

        let (emit, seen) = events();
        start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/l2")],
            h,
            format!("{r}/to"),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        let failures = end.failures.unwrap_or_default();
        assert!(
            failures[0]
                .error
                .as_deref()
                .unwrap_or("")
                .contains("copying links on a host"),
            "{failures:?}"
        );
        assert!(!remote.path().join("to/l2").exists());
    }

    /// A link that is the folder itself is caught (#4296); a FIFO is
    /// refused; the source's mode and time are kept.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aliases_specials_and_metadata() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        let r = display_path(remote.path());
        std::fs::create_dir_all(remote.path().join("a")).unwrap();
        std::os::unix::fs::symlink(remote.path().join("a"), remote.path().join("link")).unwrap();
        let h = host(remote.path()).await;
        // `link` is `a`: copying `a` into it is into itself.
        let err = start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/a")],
            h.clone(),
            format!("{r}/link"),
            events().0,
        )
        .await
        .unwrap_err();
        assert!(err.contains("into itself"), "{err}");

        // A FIFO isn't streamed (it would block): refused with why.
        let fifo = remote.path().join("pipe");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let (emit, seen) = events();
        start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/pipe")],
            End::Local,
            display_path(local.path()),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        let failures = end.failures.unwrap_or_default();
        assert!(
            failures[0]
                .error
                .as_deref()
                .unwrap_or("")
                .contains("isn't a regular file"),
            "{failures:?}"
        );

        // Mode and time come along.
        use std::os::unix::fs::PermissionsExt;
        let tool = local.path().join("tool.sh");
        std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o750)).unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::File::options()
            .write(true)
            .open(&tool)
            .unwrap()
            .set_modified(when)
            .unwrap();
        let (emit, seen) = events();
        start(
            FsOpKind::Move,
            End::Local,
            vec![display_path(&tool)],
            h,
            r.clone(),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        let landed = std::fs::metadata(remote.path().join("tool.sh")).unwrap();
        assert_eq!(landed.permissions().mode() & 0o7777, 0o750);
        assert_eq!(landed.modified().unwrap(), when);
        assert!(!tool.exists());
    }

    /// A protected folder named through a link in its parent is still that
    /// folder: refused (#4296).
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_linked_parent_does_not_spell_past_protection() {
        let remote = tempfile::tempdir().unwrap();
        let r = display_path(remote.path());
        // `etc-parent` is `/`: `etc-parent/etc` is `/etc`.
        std::os::unix::fs::symlink("/", remote.path().join("etc-parent")).unwrap();
        let h = host(remote.path()).await;
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"a").unwrap();
        let err = start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("a.txt"))],
            h.clone(),
            format!("{r}/etc-parent/etc"),
            events().0,
        )
        .await
        .unwrap_err();
        assert!(err.contains("doesn't change"), "{err}");
        let moved = start(
            FsOpKind::Move,
            h,
            vec![format!("{r}/etc-parent/etc")],
            End::Local,
            display_path(local.path()),
            events().0,
        )
        .await
        .unwrap_err();
        assert!(moved.contains("doesn't change"), "{moved}");
    }

    /// A missing source or a protected place is refused before the op starts.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bad_sources_are_refused_before_an_op_exists() {
        let remote = tempfile::tempdir().unwrap();
        let r = display_path(remote.path());
        let h = host(remote.path()).await;
        let missing = start(
            FsOpKind::Move,
            h.clone(),
            vec![format!("{r}/nope")],
            End::Local,
            "/tmp".into(),
            events().0,
        )
        .await;
        assert!(missing.unwrap_err().contains("isn't there anymore"));
        std::fs::write(remote.path().join("f"), b"f").unwrap();
        let protected = start(
            FsOpKind::Copy,
            h,
            vec![format!("{r}/f")],
            End::Local,
            "/etc".into(),
            events().0,
        )
        .await;
        assert!(protected.is_err());
    }

    #[test]
    fn one_hosts_plan_matches_the_local_rules() {
        assert!(
            under("/a/b", "/a") && under("/a", "/") && !under("/ab", "/a") && !under("/a", "/a")
        );
    }

    /// On a host a backslash is part of a name: `a\b.txt` stays itself.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_backslash_is_part_of_a_hosts_file_name() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(remote.path().join("a\\b.txt"), b"x").unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        let src = format!("{}/a\\b.txt", display_path(remote.path()));
        start(
            FsOpKind::Copy,
            h,
            vec![src],
            End::Local,
            display_path(local.path()),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(std::fs::read(local.path().join("a\\b.txt")).unwrap(), b"x");
        assert!(!local.path().join("b.txt").exists());
    }

    #[test]
    fn host_paths_join_and_split_as_posix() {
        assert_eq!(posix_join("/home/u", "a.txt"), "/home/u/a.txt");
        assert_eq!(posix_join("/", "a"), "/a");
        assert_eq!(posix_parent("/home/u/a.txt"), "/home/u");
        assert_eq!(posix_parent("/a"), "/");
        assert_eq!(posix_parent("/"), "/");
        // Names from either side: a host's path, or this computer's.
        assert_eq!(file_name("/a/b/c.txt"), "c.txt");
        assert_eq!(file_name("C:\\x\\y.md"), "y.md");
        assert_eq!(file_name("/a/b/"), "b");
    }
}
