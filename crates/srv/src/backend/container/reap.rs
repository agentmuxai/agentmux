// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Stopping a provider-CLI install whose turn was cancelled.

use std::sync::Arc;

use tokio::sync::Mutex;

use super::ContainerManager;

/// Reaps the install running in `container_name` when dropped, unless
/// [disarmed](Self::disarm). A stop drops the start-up future while the install
/// is still running in the container; nothing else would end it.
///
/// The reap runs under the same per-container lock as an install. A turn that
/// follows the stopped one therefore either reaps the orphan itself first and
/// finishes its install before this runs (which then finds nothing), or waits
/// for this: the reap never lands on the successor's install.
pub(super) struct ReapOnDrop {
    target: Option<(ContainerManager, String)>,
}

impl ReapOnDrop {
    pub(super) fn new(cm: &ContainerManager, container_name: &str) -> Self {
        Self { target: Some((cm.clone(), container_name.to_string())) }
    }

    pub(super) fn disarm(mut self) {
        self.target = None;
    }
}

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        let Some((cm, name)) = self.target.take() else { return };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let lock = {
                    let mut locks = cm.inner.cli_locks.lock().await;
                    locks.entry(name.clone()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
                };
                let held = lock.lock().await;
                cm.reap_cli_install(&name).await;
                drop(held);
                let mut locks = cm.inner.cli_locks.lock().await;
                if Arc::strong_count(&lock) <= 2 {
                    locks.remove(&name);
                }
            });
        }
    }
}
