// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The controllers that keep one agent process and a protocol session in it:
//! ACP agents (`acp`) and Muse Code over MSP (`msp`). The message paths
//! (`agentinput`, `agent.send`, reactive delivery) treat them alike: start the
//! agent again if it exited, then hand it the message.

use super::acp::AcpController;
use super::msp::MspController;
use super::Controller;

pub trait SessionAgent {
    /// Its process is running.
    fn is_running(&self) -> bool;
    /// Start it again from the pane's meta if its process exited.
    fn ensure_started(&self, block_meta: crate::backend::obj::MetaMapType) -> Result<(), String>;
    /// Send a user message as a prompt (a turn).
    fn send_message(&self, message: String, message_id: Option<&str>) -> Result<(), String>;
}

impl SessionAgent for AcpController {
    fn is_running(&self) -> bool {
        AcpController::is_running(self)
    }
    fn ensure_started(&self, block_meta: crate::backend::obj::MetaMapType) -> Result<(), String> {
        AcpController::ensure_started(self, block_meta)
    }
    fn send_message(&self, message: String, message_id: Option<&str>) -> Result<(), String> {
        AcpController::send_message(self, message, message_id)
    }
}

impl SessionAgent for MspController {
    fn is_running(&self) -> bool {
        MspController::is_running(self)
    }
    fn ensure_started(&self, block_meta: crate::backend::obj::MetaMapType) -> Result<(), String> {
        MspController::ensure_started(self, block_meta)
    }
    fn send_message(&self, message: String, message_id: Option<&str>) -> Result<(), String> {
        MspController::send_message(self, message, message_id)
    }
}

/// `ctrl` as a session agent, when it is one.
pub fn session_agent(ctrl: &dyn Controller) -> Option<&dyn SessionAgent> {
    let any = ctrl.as_any();
    if let Some(acp) = any.downcast_ref::<AcpController>() {
        return Some(acp);
    }
    any.downcast_ref::<MspController>().map(|msp| msp as &dyn SessionAgent)
}

/// Whether `controller_type` names a session agent's controller.
pub fn is_session_agent_type(controller_type: &str) -> bool {
    controller_type == super::BLOCK_CONTROLLER_ACP || controller_type == super::BLOCK_CONTROLLER_MSP
}
