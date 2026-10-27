// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! [`Env`] for the running srv: the pid table, the process table, the agent
//! registry and the audit log.

use super::pids;
use super::reader::{Env, Event};
use super::sender::{ProcView, SystemProcs};

pub struct LiveEnv;

impl Env for LiveEnv {
    fn cli_pids(&self) -> Vec<(String, u32)> {
        pids::snapshot()
    }

    fn procs(&self) -> &dyn ProcView {
        &SystemProcs
    }

    fn agent_name(&self, block_id: &str) -> Option<String> {
        crate::backend::reactive::get_global_handler().names_for_block(block_id).into_iter().next()
    }

    fn record(&self, event: Event) {
        tracing::info!(
            block_id = %event.block_id,
            msg_id = %event.msg_id,
            state = event.state,
            seen_by = event.seen_by,
            source = %event.source,
            "cli-native message"
        );
        crate::backend::reactive::get_global_handler().audit_cli_native(&event);
    }
}
