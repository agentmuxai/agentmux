// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent-anchored session zone RPCs (`agent:session:read`, `write_state`, `append_output`, `archive`,
//! `list_archives`; Option E, PR 1 of 2), moved out of `register` unchanged as named functions. A write or an
//! archive is made by the agent itself or the AgentMux window, never another agent.

use crate::backend::rpc_types::{
    AgentArchiveRow, AgentSessionAppendOutputResult, AgentSessionArchiveResult, AgentSessionReadResult,
    AgentSessionWriteStateResult, CommandAgentSessionAppendOutputData, CommandAgentSessionArchiveData,
    CommandAgentSessionListArchivesData, CommandAgentSessionReadData, CommandAgentSessionWriteStateData, RpcContext,
};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::server::actor::require_own_transcript;

pub(super) fn read_session(filestore: &FileStore, cmd: &CommandAgentSessionReadData) -> Result<AgentSessionReadResult, String> {
    let (content, modts) = crate::backend::agent_session::read_session_state(filestore, &cmd.definition_id)
        .map_err(|e| format!("agent:session:read: {e}"))?;
    Ok(AgentSessionReadResult { content, modts })
}

pub(super) fn write_state(
    filestore: &FileStore,
    mstore: &Store,
    ctx: &RpcContext,
    cmd: &CommandAgentSessionWriteStateData,
) -> Result<AgentSessionWriteStateResult, String> {
    require_own_transcript(ctx, mstore, &cmd.definition_id, "an agent's session state")?;
    let bytes = cmd.content.as_bytes();
    let bytes_written = bytes.len() as u64;
    crate::backend::agent_session::write_session_state(filestore, &cmd.definition_id, bytes)
        .map_err(|e| format!("agent:session:write_state: {e}"))?;
    Ok(AgentSessionWriteStateResult { bytes_written })
}

pub(super) fn append_output(
    filestore: &FileStore,
    mstore: &Store,
    ctx: &RpcContext,
    cmd: &CommandAgentSessionAppendOutputData,
) -> Result<AgentSessionAppendOutputResult, String> {
    require_own_transcript(ctx, mstore, &cmd.definition_id, "an agent's transcript")?;
    let bytes_written = crate::backend::agent_session::append_session_output(filestore, &cmd.definition_id, &cmd.line)
        .map_err(|e| format!("agent:session:append_output: {e}"))?;
    Ok(AgentSessionAppendOutputResult { bytes_written })
}

pub(super) fn archive(
    filestore: &FileStore,
    mstore: &Store,
    ctx: &RpcContext,
    cmd: &CommandAgentSessionArchiveData,
) -> Result<AgentSessionArchiveResult, String> {
    require_own_transcript(ctx, mstore, &cmd.definition_id, "an agent's session")?;
    let result = crate::backend::agent_session::archive_session(filestore, &cmd.definition_id)
        .map_err(|e| format!("agent:session:archive: {e}"))?;
    // Nothing to archive reads as an empty zone id and a zero time.
    let (archive_zoneid, archived_at_ms) = result.unwrap_or((String::new(), 0));
    Ok(AgentSessionArchiveResult { archive_zoneid, archived_at_ms })
}

pub(super) fn list_archives(
    filestore: &FileStore,
    cmd: &CommandAgentSessionListArchivesData,
) -> Result<Vec<AgentArchiveRow>, String> {
    let summaries = crate::backend::agent_session::list_archives(filestore, &cmd.definition_id, cmd.limit)
        .map_err(|e| format!("agent:session:list_archives: {e}"))?;
    Ok(summaries
        .into_iter()
        .map(|s| AgentArchiveRow {
            archive_zoneid: s.archive_zoneid,
            archived_at_ms: s.archived_at_ms,
            preview: s.preview,
            node_count: s.node_count,
        })
        .collect())
}
