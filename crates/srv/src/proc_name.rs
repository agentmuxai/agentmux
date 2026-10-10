// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The name `ps`, `top` and desktop system monitors show for this process on
//! Linux.
//!
//! The kernel keeps a 15-byte process name (`comm`), cut from the file name.
//! srv ships as `agentmux-srv-<version>-linux.x64`, so without this it shows
//! as `agentmux-srv-0.`, a name that changes with every release. Each role srv
//! runs in sets a stable name instead
//! (docs/reports/REPORT_PROCESS_ICONS_NAMES_AND_LABELS_2026_10_10.md §8.2).
//!
//! Only the calling thread is renamed, and the main thread's name is the
//! process's, so call this from the main thread. Elsewhere it does nothing:
//! Windows takes the name from the exe's version resource (`build.rs`), and
//! macOS has no equivalent call.

/// The backend itself.
pub const MAIN: &str = "agentmux-srv";

/// The child srv re-runs itself as to parse one attached document
/// (`backend::attachments::extract`).
pub const DOC_CHILD: &str = "agentmux-srvdoc";

/// Set this thread's name. Longer than 15 bytes is cut by the kernel.
pub fn set(name: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(c) = std::ffi::CString::new(name) {
            // SAFETY: PR_SET_NAME reads a NUL-terminated string, copying at
            // most 16 bytes; `c` outlives the call.
            unsafe {
                libc::prctl(libc::PR_SET_NAME, c.as_ptr() as libc::c_ulong, 0, 0, 0);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = name;
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn own_thread_name() -> String {
        std::fs::read_to_string("/proc/thread-self/comm").unwrap().trim_end().to_string()
    }

    // A spawned thread, so renaming it can't affect the test harness.
    #[test]
    fn sets_the_names_srv_uses() {
        std::thread::spawn(|| {
            for name in [MAIN, DOC_CHILD] {
                assert!(name.len() <= 15, "{name} would be cut by the kernel");
                set(name);
                assert_eq!(own_thread_name(), name);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_long_name_is_cut_to_fifteen_bytes() {
        std::thread::spawn(|| {
            set("agentmux-srv-0.59.18-linux.x64");
            assert_eq!(own_thread_name(), "agentmux-srv-0.");
        })
        .join()
        .unwrap();
    }
}
