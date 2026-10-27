// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Stopping a provider-CLI install whose turn was cancelled.

use super::ContainerManager;

/// Reaps the install running in `container_name` when dropped, unless
/// [disarmed](Self::disarm). A stop drops the start-up future while the install
/// is still running in the container; nothing else would end it, and the
/// next install waits for it to be gone.
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
                cm.reap_cli_install(&name).await;
            });
        }
    }
}
