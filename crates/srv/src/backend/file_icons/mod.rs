// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The system's own icons for the Hangar (Files pane): the picture the operating
//! system shows for a file type, served as a PNG the page draws in a row or tile.
//! docs/specs/SPEC_HANGAR_SYSTEM_FILE_ICONS_2026_10_27.md.
//!
//! Phase 1: Windows, by file type (an extension, or a folder). The shell is asked
//! in a helper process ([`pool`]) so a misbehaving shell extension can't take srv
//! down, the answer is kept in memory and on disk ([`cache`]), and only the PNG
//! leaves ([`crate::server`] `file_icons`). Elsewhere there is no source yet and
//! the route answers 501, which the page reads as "use the built-in icons".

mod assoc;
mod cache;
pub mod child;
mod pool;
pub mod request;
#[cfg(windows)]
mod shell_windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use request::IconRequest;

/// The argument that turns a re-run of srv into the helper (`srv __file_icons`).
pub const CHILD_ARG: &str = "__file_icons";

/// Why no picture was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconError {
    /// This operating system has no source yet.
    Unsupported,
    /// The system has no icon for it.
    NoIcon,
    /// The helper didn't answer in time and was stopped.
    Timeout,
    /// Too many requests are waiting.
    Busy,
    /// The helper broke or answered nonsense.
    Failed,
}

type Flight = Arc<tokio::sync::OnceCell<Result<Arc<Vec<u8>>, IconError>>>;

struct Service {
    pool: pool::Pool,
    mem: Mutex<cache::MemCache>,
    /// Requests being made now, so a screenful of rows of one type asks the helper once.
    flights: Mutex<HashMap<String, Flight>>,
    dir: PathBuf,
    /// Icons written since the disk cache was last trimmed.
    written: AtomicUsize,
    /// A trim is running, so only one at a time.
    trimming: AtomicBool,
}

fn service() -> &'static Service {
    static SERVICE: OnceLock<Service> = OnceLock::new();
    SERVICE.get_or_init(|| {
        let dir = crate::backend::base::get_mux_data_dir().join("cache").join("file-icons").join("v1");
        let prune_dir = dir.clone();
        std::thread::spawn(move || cache::prune(&prune_dir, cache::DISK_CAP_BYTES));
        Service {
            pool: pool::Pool::new(),
            mem: Mutex::default(),
            flights: Mutex::default(),
            dir,
            written: AtomicUsize::new(0),
            trimming: AtomicBool::new(false),
        }
    })
}

impl Service {
    /// Count a file written to the disk cache and, every [`TRIM_EVERY`] of them, trim it to its
    /// cap on a blocking thread (one trim at a time), so a long session can't outgrow the cap.
    fn note_written(&'static self) {
        if self.written.fetch_add(1, Ordering::Relaxed) % TRIM_EVERY != TRIM_EVERY - 1 {
            return;
        }
        if self.trimming.swap(true, Ordering::Acquire) {
            return;
        }
        tokio::task::spawn_blocking(move || {
            cache::prune(&self.dir, cache::DISK_CAP_BYTES);
            self.trimming.store(false, Ordering::Release);
        });
    }
}

/// Files written between trims of the disk cache.
const TRIM_EVERY: usize = 32;

/// The PNG of `req`'s icon: from memory, then disk, then the system.
pub async fn icon_png(req: &IconRequest) -> Result<Arc<Vec<u8>>, IconError> {
    if !cfg!(windows) {
        return Err(IconError::Unsupported);
    }
    let svc = service();
    let assoc = assoc::fingerprint(req);
    let key = cache::key(req, &assoc);
    if let Some(hit) = svc.mem.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(hit);
    }
    let flight = svc.flights.lock().unwrap_or_else(|e| e.into_inner()).entry(key.clone()).or_default().clone();
    let made = flight
        .get_or_init(|| async {
            let path = cache::disk_path(&svc.dir, req, &assoc);
            let png = match tokio::fs::read(&path).await {
                Ok(bytes) if !bytes.is_empty() => Arc::new(bytes),
                _ => {
                    let png = Arc::new(svc.pool.render(req).await?);
                    let bytes = png.clone();
                    tokio::spawn(async move {
                        if let Some(parent) = path.parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        let tmp = path.with_extension("tmp");
                        if tokio::fs::write(&tmp, bytes.as_slice()).await.is_ok() {
                            let _ = tokio::fs::rename(&tmp, &path).await;
                            svc.note_written();
                        }
                    });
                    png
                }
            };
            svc.mem.lock().unwrap_or_else(|e| e.into_inner()).put(key.clone(), png.clone(), cache::MEM_CAP_BYTES);
            Ok(png)
        })
        .await
        .clone();
    svc.flights.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    made
}
