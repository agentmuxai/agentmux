// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent:memory:adopt` and `agent:memory:release`: what the person confirmed
//! in the Memory pane, which of an agent's earlier accounts' memory folders to
//! adopt, or which claimed folder to release
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2 and §2.1.4).
//!
//! Both are **operator-only**: srv takes them only from the AgentMux window,
//! proven through the host. The shared auth key is in every agent's
//! environment, so a command an agent can call can't stand for a person's
//! confirmation. The window asks in a modal over the Memory pane, which an
//! agent can open but neither read nor answer
//! (SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §6 row 8). The choices
//! are only `(list_id, index, dir_hash)` from a list the server issued
//! (`agent:memory:adoption_list`, `agent:memory:claims`), never a path.

use std::sync::Arc;

use crate::backend::memory::adopt::{self, AdoptError, AdoptReport};
use crate::backend::memory::release::{self, ReleaseError};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandNativeMemoryAdoptData, CommandNativeMemoryReleaseData, NativeMemoryReleaseResult, RpcContext,
    COMMAND_NATIVE_MEMORY_ADOPT, COMMAND_NATIVE_MEMORY_RELEASE,
};
use crate::backend::transcript::damage;

use super::AppState;

const WHY: &str = "only the person adopts memory or releases a memory folder, in the Memory pane";

/// The errors start with a stable word the pane reads: "changed" means list again.
async fn native_memory_adopt(state: &AppState, cmd: CommandNativeMemoryAdoptData) -> Result<AdoptReport, String> {
    if cmd.choices.is_empty() {
        return Err("bad-choice: nothing chosen".to_string());
    }
    let fs = damage::global_for(&damage::memory_zone(&cmd.agent_id))
        .ok_or_else(|| "store: the memory record store is unavailable, or its record is damaged".to_string())?;
    let mstore = state.mstore.clone();
    let choices: Vec<(usize, String)> = cmd.choices.into_iter().map(|c| (c.index, c.dir_hash)).collect();
    let result =
        tokio::task::spawn_blocking(move || adopt::adopt(&fs, &mstore, &cmd.agent_id, &cmd.list_id, &choices)).await;
    match result {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(AdoptError::Changed { index })) => {
            Err(format!("changed: folder {index} changed since the list was issued; list again"))
        }
        Ok(Err(AdoptError::UnknownList)) => {
            Err("unknown-list: the list expired or is not this agent's; list again".to_string())
        }
        Ok(Err(AdoptError::BadChoice)) => Err("bad-choice".to_string()),
        Ok(Err(AdoptError::Busy)) => Err("busy: the agent's memory is being synced; try again".to_string()),
        Ok(Err(AdoptError::Store(e))) => {
            tracing::warn!("[memory-adopt] adopt failed: {e}");
            Err("store error".to_string())
        }
        Err(e) => Err(format!("agent:memory:adopt: {e}")),
    }
}

async fn native_memory_release(cmd: CommandNativeMemoryReleaseData) -> Result<NativeMemoryReleaseResult, String> {
    let fs = damage::global_for(&damage::memory_zone(&cmd.agent_id))
        .ok_or_else(|| "store: the memory record store is unavailable, or its record is damaged".to_string())?;
    let result =
        tokio::task::spawn_blocking(move || release::release(&fs, &cmd.agent_id, &cmd.list_id, cmd.index)).await;
    match result {
        Ok(Ok(released)) => Ok(NativeMemoryReleaseResult { released }),
        Ok(Err(ReleaseError::UnknownList)) => {
            Err("unknown-list: the list expired or is not this agent's; list again".to_string())
        }
        Ok(Err(ReleaseError::BadChoice)) => Err("bad-choice".to_string()),
        Ok(Err(ReleaseError::Store(e))) => {
            tracing::warn!("[memory-adopt] release failed: {e}");
            Err("store error".to_string())
        }
        Err(e) => Err(format!("agent:memory:release: {e}")),
    }
}

/// `agent:memory:adopt`, from the window only.
async fn adopt_from(state: &AppState, ctx: &RpcContext, cmd: CommandNativeMemoryAdoptData) -> Result<AdoptReport, String> {
    ctx.operator_only(WHY)?;
    native_memory_adopt(state, cmd).await
}

/// `agent:memory:release`, from the window only.
async fn release_from(ctx: &RpcContext, cmd: CommandNativeMemoryReleaseData) -> Result<NativeMemoryReleaseResult, String> {
    ctx.operator_only(WHY)?;
    native_memory_release(cmd).await
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_NATIVE_MEMORY_ADOPT, move |cmd: CommandNativeMemoryAdoptData, ctx| {
        let s = s.clone();
        async move { adopt_from(&s, &ctx, cmd).await }
    });
    engine.register_typed(COMMAND_NATIVE_MEMORY_RELEASE, |cmd: CommandNativeMemoryReleaseData, ctx| async move {
        release_from(&ctx, cmd).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::NativeMemoryAdoptChoice;
    use crate::server::tests::test_state;

    fn adopt_cmd(choices: usize) -> CommandNativeMemoryAdoptData {
        let choices = (0..choices).map(|index| NativeMemoryAdoptChoice { index, dir_hash: "h".into() }).collect();
        CommandNativeMemoryAdoptData { agent_id: "a".into(), list_id: "l".into(), choices }
    }

    fn release_cmd() -> CommandNativeMemoryReleaseData {
        CommandNativeMemoryReleaseData { agent_id: "a".into(), list_id: "l".into(), index: 0 }
    }

    /// The attack this closes: an agent holds the instance auth key (every agent's
    /// environment does) and calls adopt itself, skipping the person's
    /// confirmation in the Memory pane.
    #[tokio::test]
    async fn an_agent_or_an_unproven_connection_cannot_adopt_or_release() {
        let state = test_state();
        let agent = RpcContext { agent_id: "worker".to_string(), operator_proven: true, ..Default::default() };
        for ctx in [agent, RpcContext::default()] {
            let refused = adopt_from(&state, &ctx, adopt_cmd(1)).await.unwrap_err();
            assert!(refused.starts_with("FORBIDDEN"), "{refused}");
            let refused = release_from(&ctx, release_cmd()).await.unwrap_err();
            assert!(refused.starts_with("FORBIDDEN"), "{refused}");
        }
    }

    #[tokio::test]
    async fn the_window_reaches_adopt_and_release() {
        let ctx = RpcContext::operator_for_tests();
        let e = adopt_from(&test_state(), &ctx, adopt_cmd(1)).await.unwrap_err();
        assert!(!e.starts_with("FORBIDDEN"), "{e}");
        let e = release_from(&ctx, release_cmd()).await.unwrap_err();
        assert!(!e.starts_with("FORBIDDEN"), "{e}");
    }

    #[tokio::test]
    async fn adopting_nothing_is_refused() {
        let e = adopt_from(&test_state(), &RpcContext::operator_for_tests(), adopt_cmd(0)).await.unwrap_err();
        assert_eq!(e, "bad-choice: nothing chosen");
    }
}
