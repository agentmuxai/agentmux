// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `attachments.ingest` / `attachments.cancel` / `attachments.info` — image
//! attachments in the agent composer
//! (docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6). The work
//! itself lives in `backend::attachments`; these only adapt it to the RPC
//! engine. Typed registration, so `frontend/types/rpc/` gets the bindings.

use super::*;
use crate::backend::attachments;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let svc = attachments::init(state.broker.clone(), state.config_watcher.clone());

    let ingest_svc = svc.clone();
    engine.register_typed(
        COMMAND_ATTACHMENTS_INGEST,
        move |req: CommandAttachmentsIngestData, _ctx| {
            let svc = ingest_svc.clone();
            async move {
                if req.batch_id.trim().is_empty() {
                    return Err("attachments.ingest: batch_id is required".to_string());
                }
                // Classification stats files and reads a few header bytes
                // each; a folder drop can touch thousands, so keep it off
                // the async workers. Processing then runs in the background.
                let handle = tokio::runtime::Handle::current();
                tokio::task::spawn_blocking(move || {
                    let _guard = handle.enter();
                    svc.ingest(req)
                })
                .await
                .map_err(|e| format!("attachments.ingest: {e}"))
            }
        },
    );

    let cancel_svc = svc.clone();
    engine.register_typed(
        COMMAND_ATTACHMENTS_CANCEL,
        move |req: CommandAttachmentsCancelData, _ctx| {
            let svc = cancel_svc.clone();
            async move {
                svc.cancel(&req.batch_id);
                Ok(serde_json::Value::Null)
            }
        },
    );

    let copy_svc = svc.clone();
    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_ATTACHMENTS_COPY_TO_WORKDIR,
        move |req: CommandAttachmentsCopyToWorkdirData, _ctx| {
            let svc = copy_svc.clone();
            let mstore = mstore.clone();
            async move {
                let block = mstore
                    .get::<Block>(&req.block_id)
                    .map_err(|e| format!("attachments.copy-to-workdir: {e}"))?
                    .ok_or_else(|| "attachments.copy-to-workdir: no such pane".to_string())?;
                let cwd = obj::meta_get_string(&block.meta, "cmd:cwd", "");
                if cwd.is_empty() {
                    return Err("No working folder is set for this agent.".to_string());
                }
                let path = tokio::task::spawn_blocking(move || {
                    svc.store()
                        .copy_original_to(&req.id, std::path::Path::new(&cwd), &req.name)
                })
                .await
                .map_err(|e| format!("attachments.copy-to-workdir: {e}"))?
                .map_err(|e| format!("Couldn't copy the file into the working folder: {e}"))?;
                Ok(AttachmentsCopyToWorkdirResult {
                    path: path.to_string_lossy().into_owned(),
                })
            }
        },
    );

    let info_svc = svc;
    engine.register_typed(
        COMMAND_ATTACHMENTS_INFO,
        move |req: CommandAttachmentsInfoData, _ctx| {
            let svc = info_svc.clone();
            async move {
                let items = svc.info(&req.ids).await;
                Ok(AttachmentsInfoResult { items })
            }
        },
    );
}
