// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One agent's sessions from its record, for Warden's History section
//! (`history.agent_sessions`).

use super::HistoryService;

impl HistoryService {
    /// Agent `uid`'s sessions as its record holds them (Warden's History
    /// section, `history.agent_sessions`), most recent first. The first read
    /// of a zone scans it; later reads only what was appended.
    pub fn agent_sessions(&self, uid: &str) -> crate::backend::rpc_types::HistoryAgentSessions {
        let (sessions, problems) = self.records.sessions_of(uid);
        let sessions = sessions
            .into_iter()
            .filter(|s| !s.ranges.is_empty())
            .map(|s| crate::backend::rpc_types::HistorySession {
                bytes: s.ranges.iter().map(|(_, from, to)| (to - from).max(0) as u64).sum(),
                archived: s.ranges.iter().all(|(zone, _, _)| zone.contains(":archive:")),
                session_id: s.session_id,
                cwd: s.cwd.unwrap_or_default(),
                first_ms: s.first_ms.max(0) as u64,
                last_ms: s.last_ms.max(0) as u64,
            })
            .collect();
        crate::backend::rpc_types::HistoryAgentSessions { sessions, problems }
    }
}
