// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading a package out of a container agent's workspace without ever following a link.
//!
//! The workspace is the agent's to change while it is read. Checking a path and then opening it leaves a
//! gap in which a folder on the way is swapped for a link to somewhere else, so every step here is made
//! relative to a folder already open (`openat` with `O_NOFOLLOW`): what is open is what was named, and
//! a link anywhere on the way is an error rather than a detour.

use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path};

/// What a folder entry is, without following it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Dir,
    Link,
    Other,
}

fn cstring(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a path with a NUL byte"))
}

fn owned(fd: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: `fd` is a descriptor `open`/`openat` just returned and nothing else owns.
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd) }) }
}

/// `name` in `dir`, opened with `flags`; a link in its place is an error.
fn open_in(dir: &OwnedFd, name: &OsStr, flags: i32) -> io::Result<OwnedFd> {
    let name = cstring(name.as_bytes())?;
    // SAFETY: `name` is a NUL-terminated string that outlives the call.
    owned(unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })
}

/// The folder `path`, which lies under `root` (a canonical path), opened one component at a time.
pub(crate) fn dir_beneath(root: &Path, path: &Path) -> io::Result<OwnedFd> {
    let outside = || io::Error::new(io::ErrorKind::PermissionDenied, "outside the workspace");
    let rel = path.strip_prefix(root).map_err(|_| outside())?;
    let root = cstring(root.as_os_str().as_bytes())?;
    // SAFETY: `root` is a NUL-terminated string that outlives the call.
    let mut dir = owned(unsafe { libc::open(root.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) })?;
    for part in rel.components() {
        match part {
            Component::Normal(name) => dir = open_in(&dir, name, libc::O_DIRECTORY)?,
            Component::CurDir => {}
            _ => return Err(outside()),
        }
    }
    Ok(dir)
}

/// The sub-folder `name` of `dir`.
pub(crate) fn subdir(dir: &OwnedFd, name: &OsStr) -> io::Result<OwnedFd> {
    open_in(dir, name, libc::O_DIRECTORY)
}

/// The regular file `name` of `dir`, ready to read.
pub(crate) fn file_in(dir: &OwnedFd, name: &OsStr) -> io::Result<std::fs::File> {
    // `O_NONBLOCK` so a pipe put in the file's place is refused below, not waited on.
    let file = std::fs::File::from(open_in(dir, name, libc::O_NONBLOCK)?);
    if file.metadata()?.is_file() { Ok(file) } else { Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file")) }
}

/// The regular file `path`, which lies under `root`.
pub(crate) fn file_beneath(root: &Path, path: &Path) -> io::Result<std::fs::File> {
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    file_in(&dir_beneath(root, path.parent().unwrap_or(root))?, name)
}

/// What is in `dir`, each entry's kind taken without following it.
pub(crate) fn entries(dir: &OwnedFd) -> io::Result<Vec<(OsString, Kind)>> {
    // SAFETY: `dup` returns a new descriptor that `fdopendir` then owns; on failure it is closed here.
    let copy = unsafe { libc::dup(dir.as_raw_fd()) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { libc::fdopendir(copy) };
    if stream.is_null() {
        let e = io::Error::last_os_error();
        unsafe { libc::close(copy) };
        return Err(e);
    }
    // The duplicate shares the folder's read position; start from the top.
    unsafe { libc::rewinddir(stream) };
    let mut found = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: `d_name` of a returned entry is a NUL-terminated string valid until the next `readdir`.
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes().to_vec();
        if name == b"." || name == b".." {
            continue;
        }
        let c = cstring(&name)?;
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(dir.as_raw_fd(), c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::closedir(stream) };
            return Err(e);
        }
        let kind = match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => Kind::File,
            libc::S_IFDIR => Kind::Dir,
            libc::S_IFLNK => Kind::Link,
            _ => Kind::Other,
        };
        found.push((OsString::from_vec(name), kind));
    }
    unsafe { libc::closedir(stream) };
    Ok(found)
}
