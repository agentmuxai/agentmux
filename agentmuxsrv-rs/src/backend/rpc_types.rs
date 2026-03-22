// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! RPC wire format types: Rust equivalents of Go structs from
//! pkg/wshutil/wshrpc.go and pkg/wshrpc/wshrpctypes.go.

#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::oref::ORef;
use super::waveobj::{Block, MetaMapType, Workspace};

// ---- RpcMessage wire format ----

/// Matches Go's `wshutil.RpcMessage` from pkg/wshutil/wshrpc.go.
/// This is the on-the-wire JSON envelope for all RPC communication.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reqid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resid: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub timeout: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub authtoken: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cont: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub datatype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcMessage {
    pub fn is_rpc_request(&self) -> bool {
        !self.command.is_empty() || !self.reqid.is_empty()
    }

    /// Validates the packet structure. Matches Go's `RpcMessage.Validate()`.
    pub fn validate(&self) -> Result<(), String> {
        if !self.reqid.is_empty() && !self.resid.is_empty() {
            return Err("request packets may not have both reqid and resid set".into());
        }
        if self.cancel {
            if !self.command.is_empty() {
                return Err("cancel packets may not have command set".into());
            }
            if self.reqid.is_empty() && self.resid.is_empty() {
                return Err("cancel packets must have reqid or resid set".into());
            }
            if self.data.is_some() {
                return Err("cancel packets may not have data set".into());
            }
            return Ok(());
        }
        if !self.command.is_empty() {
            if !self.resid.is_empty() {
                return Err("command packets may not have resid set".into());
            }
            if !self.error.is_empty() {
                return Err("command packets may not have error set".into());
            }
            if !self.datatype.is_empty() {
                return Err("command packets may not have datatype set".into());
            }
            return Ok(());
        }
        if !self.reqid.is_empty() {
            if self.resid.is_empty() {
                return Err("request packets must have resid set".into());
            }
            if self.timeout != 0 {
                return Err("non-command request packets may not have timeout set".into());
            }
            return Ok(());
        }
        if !self.resid.is_empty() {
            if !self.command.is_empty() {
                return Err("response packets may not have command set".into());
            }
            if self.reqid.is_empty() {
                return Err("response packets must have reqid set".into());
            }
            if self.timeout != 0 {
                return Err("response packets may not have timeout set".into());
            }
            return Ok(());
        }
        Err("invalid packet: must have command, reqid, or resid set".into())
    }
}

// ---- Size/type constants (match Go) ----

pub const MAX_FILE_SIZE: usize = 50 * 1024 * 1024; // 50M
pub const MAX_DIR_SIZE: usize = 1024;
pub const FILE_CHUNK_SIZE: usize = 64 * 1024;
pub const DIR_CHUNK_SIZE: usize = 128;

pub const LOCAL_CONN_NAME: &str = "local";

// ---- RPC type constants ----

pub const RPC_TYPE_CALL: &str = "call";
pub const RPC_TYPE_RESPONSE_STREAM: &str = "responsestream";
pub const RPC_TYPE_STREAMING_REQUEST: &str = "streamingrequest";
pub const RPC_TYPE_COMPLEX: &str = "complex";

// ---- CreateBlock action constants ----

pub const CREATE_BLOCK_ACTION_REPLACE: &str = "replace";
pub const CREATE_BLOCK_ACTION_SPLIT_UP: &str = "splitup";
pub const CREATE_BLOCK_ACTION_SPLIT_DOWN: &str = "splitdown";
pub const CREATE_BLOCK_ACTION_SPLIT_LEFT: &str = "splitleft";
pub const CREATE_BLOCK_ACTION_SPLIT_RIGHT: &str = "splitright";

// ---- Command constants (match Go's wshrpc.Command_* constants) ----

// Special commands
pub const COMMAND_AUTHENTICATE: &str = "authenticate";
pub const COMMAND_AUTHENTICATE_TOKEN: &str = "authenticatetoken";
pub const COMMAND_DISPOSE: &str = "dispose";
pub const COMMAND_ROUTE_ANNOUNCE: &str = "routeannounce";
pub const COMMAND_ROUTE_UNANNOUNCE: &str = "routeunannounce";

// Core commands
pub const COMMAND_MESSAGE: &str = "message";
pub const COMMAND_GET_META: &str = "getmeta";
pub const COMMAND_SET_META: &str = "setmeta";
pub const COMMAND_SET_VIEW: &str = "setview";

// Controller commands
pub const COMMAND_CONTROLLER_INPUT: &str = "controllerinput";
pub const COMMAND_CONTROLLER_RESTART: &str = "controllerrestart";
pub const COMMAND_CONTROLLER_STOP: &str = "controllerstop";
pub const COMMAND_CONTROLLER_RESYNC: &str = "controllerresync";

// Subprocess agent commands
pub const COMMAND_SUBPROCESS_SPAWN: &str = "subprocessspawn";
pub const COMMAND_AGENT_INPUT: &str = "agentinput";
pub const COMMAND_AGENT_STOP: &str = "agentstop";
pub const COMMAND_WRITE_AGENT_CONFIG: &str = "writeagentconfig";
pub const COMMAND_RESOLVE_CLI: &str = "resolvecli";
pub const COMMAND_CHECK_CLI_AUTH: &str = "checkcliauth";

// Block commands
pub const COMMAND_MKDIR: &str = "mkdir";
pub const COMMAND_RESOLVE_IDS: &str = "resolveids";
pub const COMMAND_BLOCK_INFO: &str = "blockinfo";
pub const COMMAND_BLOCKS_LIST: &str = "blockslist";
pub const COMMAND_CREATE_BLOCK: &str = "createblock";
pub const COMMAND_DELETE_BLOCK: &str = "deleteblock";

// File commands
pub const COMMAND_FILE_WRITE: &str = "filewrite";
pub const COMMAND_FILE_READ: &str = "fileread";
pub const COMMAND_FILE_READ_STREAM: &str = "filereadstream";
pub const COMMAND_FILE_MOVE: &str = "filemove";
pub const COMMAND_FILE_COPY: &str = "filecopy";
pub const COMMAND_FILE_STREAM_TAR: &str = "filestreamtar";
pub const COMMAND_FILE_APPEND: &str = "fileappend";
pub const COMMAND_FILE_APPEND_IJSON: &str = "fileappendijson";
pub const COMMAND_FILE_JOIN: &str = "filejoin";
pub const COMMAND_FILE_SHARE_CAPABILITY: &str = "filesharecapability";

// Event commands
pub const COMMAND_EVENT_PUBLISH: &str = "eventpublish";
pub const COMMAND_EVENT_RECV: &str = "eventrecv";
pub const COMMAND_EVENT_SUB: &str = "eventsub";
pub const COMMAND_EVENT_UNSUB: &str = "eventunsub";
pub const COMMAND_EVENT_UNSUB_ALL: &str = "eventunsuball";
pub const COMMAND_EVENT_READ_HISTORY: &str = "eventreadhistory";

// Stream/test commands
pub const COMMAND_STREAM_TEST: &str = "streamtest";
pub const COMMAND_STREAM_WAVE_AI: &str = "streamwaveai";
pub const COMMAND_STREAM_CPU_DATA: &str = "streamcpudata";
pub const COMMAND_TEST: &str = "test";

// Config commands
pub const COMMAND_SET_CONFIG: &str = "setconfig";
pub const COMMAND_SET_CONNECTIONS_CONFIG: &str = "connectionsconfig";
pub const COMMAND_GET_FULL_CONFIG: &str = "getfullconfig";

// Remote commands
pub const COMMAND_REMOTE_STREAM_FILE: &str = "remotestreamfile";
pub const COMMAND_REMOTE_TAR_STREAM: &str = "remotetarstream";
pub const COMMAND_REMOTE_FILE_INFO: &str = "remotefileinfo";
pub const COMMAND_REMOTE_FILE_TOUCH: &str = "remotefiletouch";
pub const COMMAND_REMOTE_WRITE_FILE: &str = "remotewritefile";
pub const COMMAND_REMOTE_FILE_DELETE: &str = "remotefiledelete";
pub const COMMAND_REMOTE_FILE_JOIN: &str = "remotefilejoin";
pub const COMMAND_REMOTE_MKDIR: &str = "remotemkdir";
pub const COMMAND_REMOTE_GET_INFO: &str = "remotegetinfo";
pub const COMMAND_REMOTE_INSTALL_RC_FILES: &str = "remoteinstallrcfiles";

// Info/activity commands
pub const COMMAND_APP_INFO: &str = "waveinfo";
pub const COMMAND_WSH_ACTIVITY: &str = "wshactivity";
pub const COMMAND_ACTIVITY: &str = "activity";
pub const COMMAND_GET_VAR: &str = "getvar";
pub const COMMAND_SET_VAR: &str = "setvar";

// Connection commands
pub const COMMAND_CONN_STATUS: &str = "connstatus";
pub const COMMAND_WSL_STATUS: &str = "wslstatus";
pub const COMMAND_CONN_ENSURE: &str = "connensure";
pub const COMMAND_CONN_REINSTALL_WSH: &str = "connreinstallwsh";
pub const COMMAND_CONN_CONNECT: &str = "connconnect";
pub const COMMAND_CONN_DISCONNECT: &str = "conndisconnect";
pub const COMMAND_CONN_LIST: &str = "connlist";
pub const COMMAND_CONN_LIST_AWS: &str = "connlistaws";
pub const COMMAND_WSL_LIST: &str = "wsllist";
pub const COMMAND_WSL_DEFAULT_DISTRO: &str = "wsldefaultdistro";
pub const COMMAND_DISMISS_WSH_FAIL: &str = "dismisswshfail";
pub const COMMAND_CONN_UPDATE_WSH: &str = "updatewsh";

// Workspace commands
pub const COMMAND_WORKSPACE_LIST: &str = "workspacelist";

// UI commands
pub const COMMAND_WEB_SELECTOR: &str = "webselector";
pub const COMMAND_NOTIFY: &str = "notify";
pub const COMMAND_FOCUS_WINDOW: &str = "focuswindow";
pub const COMMAND_GET_UPDATE_CHANNEL: &str = "getupdatechannel";

// VDom commands
pub const COMMAND_VDOM_CREATE_CONTEXT: &str = "vdomcreatecontext";
pub const COMMAND_VDOM_ASYNC_INITIATION: &str = "vdomasyncinitiation";
pub const COMMAND_VDOM_RENDER: &str = "vdomrender";
pub const COMMAND_VDOM_URL_REQUEST: &str = "vdomurlrequest";

// AI commands
pub const COMMAND_AI_SEND_MESSAGE: &str = "aisendmessage";
pub const COMMAND_AI_ENABLE_TELEMETRY: &str = "waveaienabletelemetry";
pub const COMMAND_GET_AI_CHAT: &str = "getwaveaichat";
pub const COMMAND_GET_AI_RATE_LIMIT: &str = "getwaveairatelimit";
pub const COMMAND_AI_TOOL_APPROVE: &str = "waveaitoolapprove";
pub const COMMAND_AI_ADD_CONTEXT: &str = "waveaiaddcontext";

// Screenshot
pub const COMMAND_CAPTURE_BLOCK_SCREENSHOT: &str = "captureblockscreenshot";

// RT info
pub const COMMAND_GET_RT_INFO: &str = "getrtinfo";
pub const COMMAND_SET_RT_INFO: &str = "setrtinfo";

// Terminal
pub const COMMAND_TERM_GET_SCROLLBACK_LINES: &str = "termgetscrollbacklines";

// Forge
pub const COMMAND_LIST_FORGE_AGENTS: &str = "listforgeagents";
pub const COMMAND_CREATE_FORGE_AGENT: &str = "createforgeagent";
pub const COMMAND_UPDATE_FORGE_AGENT: &str = "updateforgeagent";
pub const COMMAND_DELETE_FORGE_AGENT: &str = "deleteforgeagent";
pub const COMMAND_GET_FORGE_CONTENT: &str = "getforgecontent";
pub const COMMAND_SET_FORGE_CONTENT: &str = "setforgecontent";
pub const COMMAND_GET_ALL_FORGE_CONTENT: &str = "getallforgecontent";

// Forge Skills
pub const COMMAND_LIST_FORGE_SKILLS: &str = "listforgeskills";
pub const COMMAND_CREATE_FORGE_SKILL: &str = "createforgeskill";
pub const COMMAND_UPDATE_FORGE_SKILL: &str = "updateforgeskill";
pub const COMMAND_DELETE_FORGE_SKILL: &str = "deleteforgeskill";

// Forge History
pub const COMMAND_APPEND_FORGE_HISTORY: &str = "appendforgehistory";
pub const COMMAND_LIST_FORGE_HISTORY: &str = "listforgehistory";
pub const COMMAND_SEARCH_FORGE_HISTORY: &str = "searchforgehistory";

// Forge Import
pub const COMMAND_IMPORT_FORGE_FROM_CLAW: &str = "importforgefromclaw";

// Forge Seed
pub const COMMAND_RESEED_FORGE_AGENTS: &str = "reseedforgeagents";

// ---- Client type constants ----

pub const CLIENT_TYPE_CONN_SERVER: &str = "connserver";
pub const CLIENT_TYPE_BLOCK_CONTROLLER: &str = "blockcontroller";

// ---- Command data types ----

/// Matches Go's `CommandGetMetaData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetMetaData {
    pub oref: ORef,
}

/// Matches Go's `CommandSetMetaData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSetMetaData {
    pub oref: ORef,
    pub meta: MetaMapType,
}

/// Matches Go's `CommandMessageData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMessageData {
    #[serde(default)]
    pub oref: ORef,
    pub message: String,
}

/// Matches Go's `CommandAuthenticateRtnData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandAuthenticateRtnData {
    pub routeid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub authtoken: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initscripttext: String,
}

/// Matches Go's `CommandAuthenticateTokenData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAuthenticateTokenData {
    pub token: String,
}

/// Matches Go's `CommandDisposeData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDisposeData {
    pub routeid: String,
}

/// Matches Go's `CommandResolveIdsData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResolveIdsData {
    #[serde(default)]
    pub blockid: String,
    pub ids: Vec<String>,
}

/// Matches Go's `CommandResolveIdsRtnData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResolveIdsRtnData {
    pub resolvedids: HashMap<String, ORef>,
}

/// Matches Go's `CommandCreateBlockData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandCreateBlockData {
    #[serde(default)]
    pub tabid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockdef: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtopts: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub magnified: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub targetblockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub targetaction: String,
}

/// Matches Go's `CommandDeleteBlockData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteBlockData {
    pub blockid: String,
}

/// Matches Go's `CommandBlockSetViewData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBlockSetViewData {
    pub blockid: String,
    pub view: String,
}

/// Matches Go's `CommandControllerResyncData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandControllerResyncData {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forcerestart: bool,
    #[serde(default)]
    pub tabid: String,
    #[serde(default)]
    pub blockid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtopts: Option<serde_json::Value>,
}

/// Matches Go's `CommandBlockInputData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandBlockInputData {
    pub blockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inputdata64: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signame: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termsize: Option<serde_json::Value>,
}

// ---- Subprocess agent command data types ----

/// Data for SubprocessSpawnCommand — spawn agent CLI for a single turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSubprocessSpawnData {
    pub blockid: String,
    pub tabid: String,
    pub cli_command: String,
    #[serde(default)]
    pub cli_args: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub env_vars: std::collections::HashMap<String, String>,
    /// The user's JSON message to write to subprocess stdin.
    pub message: String,
}

/// Data for AgentInputCommand — send a follow-up message (re-spawns with --resume).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentInputData {
    pub blockid: String,
    /// The user's JSON message string.
    pub message: String,
}

/// Data for AgentStopCommand — stop the running subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentStopData {
    pub blockid: String,
    #[serde(default)]
    pub force: bool,
}

/// A file to write as part of agent config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigFile {
    pub path: String,
    pub content: String,
}

/// Data for WriteAgentConfigCommand — write config files atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandWriteAgentConfigData {
    /// Agent working directory where files are written.
    pub working_dir: String,
    /// Files to write (path relative to working_dir, content).
    pub files: Vec<AgentConfigFile>,
}

/// Data for ResolveCliCommand — detect or install a CLI tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResolveCliData {
    /// Provider ID (e.g. "claude", "codex", "gemini")
    pub provider_id: String,
    /// CLI command name (e.g. "claude")
    pub cli_command: String,
    /// npm package name for fallback install (e.g. "@anthropic-ai/claude-code")
    pub npm_package: String,
    /// Version to install ("latest" or specific version)
    pub pinned_version: String,
    /// Windows install command (e.g. "irm https://claude.ai/install.ps1 | iex")
    #[serde(default)]
    pub windows_install_command: String,
    /// Unix install command (e.g. "curl -fsSL https://claude.ai/install.sh | bash")
    #[serde(default)]
    pub unix_install_command: String,
    /// Block ID — if set, install log lines are emitted as cli:install:log events scoped to this block
    #[serde(default)]
    pub block_id: Option<String>,
}

/// Result from ResolveCliCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveCliResult {
    /// Absolute path to the CLI binary
    pub cli_path: String,
    /// CLI version string
    pub version: String,
    /// How it was resolved: "path", "local_install", "installed"
    pub source: String,
}

/// Data for CheckCliAuthCommand — check if CLI is authenticated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCheckCliAuthData {
    /// Absolute path to CLI binary
    pub cli_path: String,
    /// Auth check args (e.g. ["auth", "status", "--json"])
    pub auth_check_args: Vec<String>,
    /// Environment variables to set when running the auth check (e.g. CLAUDE_CONFIG_DIR).
    /// Must match the env vars used when spawning the actual subprocess so the check
    /// reads credentials from the same isolated directory.
    #[serde(default)]
    pub auth_env: std::collections::HashMap<String, String>,
}

/// Result from CheckCliAuthCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckCliAuthResult {
    pub authenticated: bool,
    pub email: Option<String>,
    pub auth_method: Option<String>,
    /// Raw stdout from auth check command
    pub raw_output: String,
}

/// Input for RunCliLoginCommand — spawns the CLI login flow and extracts the OAuth URL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRunCliLoginData {
    pub cli_path: String,
    pub login_args: Vec<String>,
    #[serde(default)]
    pub auth_env: HashMap<String, String>,
}

/// Result from RunCliLoginCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCliLoginResult {
    /// OAuth URL extracted from the CLI's output (open in browser)
    pub auth_url: Option<String>,
    pub raw_output: String,
}

/// Matches Go's `FileDataAt`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDataAt {
    pub offset: i64,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub size: usize,
}

/// Matches Go's `FileData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<FileInfo>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<FileInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<FileDataAt>,
}

/// Matches Go's `FileInfo`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileInfo {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notfound: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opts: Option<FileOpts>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub modtime: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isdir: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mimetype: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub readonly: bool,
}

/// Matches Go's `FileOpts`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileOpts {
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub maxsize: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub circular: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ijson: bool,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub ijsonbudget: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncate: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
}

/// Matches Go's `CommandEventReadHistoryData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEventReadHistoryData {
    pub event: String,
    pub scope: String,
    #[serde(default)]
    pub maxitems: usize,
}

/// Matches Go's `CommandWaitForRouteData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandWaitForRouteData {
    pub routeid: String,
    #[serde(default)]
    pub waitms: i64,
}

/// Matches Go's `BlockInfoData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfoData {
    pub blockid: String,
    pub tabid: String,
    pub workspaceid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<FileInfo>>,
}

/// Matches Go's `WaveInfoData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaveInfoData {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub clientid: String,
    #[serde(default)]
    pub buildtime: String,
    #[serde(default)]
    pub configdir: String,
    #[serde(default)]
    pub datadir: String,
}

/// Matches Go's `WorkspaceInfoData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfoData {
    pub windowid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspacedata: Option<Workspace>,
}

/// Matches Go's `ConnStatus`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnStatus {
    pub status: String,
    #[serde(default)]
    pub wshenabled: bool,
    #[serde(default)]
    pub connection: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub hasconnected: bool,
    #[serde(default)]
    pub activeconnnum: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wsherror: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nowshreason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wshversion: String,
}

/// Matches Go's `WaveNotificationOptions`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaveNotificationOptions {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub silent: bool,
}

/// Matches Go's `RpcOpts`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcOpts {
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub timeout: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub noresponse: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route: String,
}

/// Matches Go's `RpcContext`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcContext {
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "ctype")]
    pub client_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tabid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub conn: String,
}

/// Matches Go's `CommandVarData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandVarData {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub val: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
    #[serde(default)]
    pub zoneid: String,
    #[serde(default)]
    pub filename: String,
}

/// Matches Go's `CommandVarResponseData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandVarResponseData {
    pub key: String,
    #[serde(default)]
    pub val: String,
    #[serde(default)]
    pub exists: bool,
}

/// Matches Go's `TimeSeriesData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesData {
    pub ts: i64,
    pub values: HashMap<String, f64>,
}

/// Matches Go's `RemoteInfo`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteInfo {
    #[serde(default)]
    pub clientarch: String,
    #[serde(default)]
    pub clientos: String,
    #[serde(default)]
    pub clientversion: String,
    #[serde(default)]
    pub shell: String,
}

// ---- Helper functions ----

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

// ---- Forge command data types ----

/// Input for createforgeagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCreateForgeAgentData {
    pub name: String,
    #[serde(default = "default_forge_icon")]
    pub icon: String,
    pub provider: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub provider_flags: String,
    #[serde(default)]
    pub auto_start: i64,
    #[serde(default)]
    pub restart_on_crash: i64,
    #[serde(default)]
    pub idle_timeout_minutes: i64,
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
}

fn default_agent_type() -> String {
    "standalone".to_string()
}

fn default_forge_icon() -> String {
    "✦".to_string()
}

/// Input for updateforgeagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUpdateForgeAgentData {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub provider: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub provider_flags: String,
    #[serde(default)]
    pub auto_start: i64,
    #[serde(default)]
    pub restart_on_crash: i64,
    #[serde(default)]
    pub idle_timeout_minutes: i64,
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
}

/// Input for deleteforgeagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteForgeAgentData {
    pub id: String,
}

/// Input for getforgecontent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetForgeContentData {
    pub agent_id: String,
    pub content_type: String,
}

/// Input for setforgecontent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSetForgeContentData {
    pub agent_id: String,
    pub content_type: String,
    pub content: String,
}

/// Input for getallforgecontent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetAllForgeContentData {
    pub agent_id: String,
}

// ---- Forge Skills command data types ----

/// Input for listforgeskills
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListForgeSkillsData {
    pub agent_id: String,
}

/// Input for createforgeskill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCreateForgeSkillData {
    pub agent_id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

fn default_skill_type() -> String {
    "prompt".to_string()
}

/// Input for updateforgeskill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUpdateForgeSkillData {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

/// Input for deleteforgeskill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteForgeSkillData {
    pub id: String,
}

// ---- Forge History command data types ----

/// Input for appendforgehistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAppendForgeHistoryData {
    pub agent_id: String,
    pub entry: String,
}

/// Input for listforgehistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListForgeHistoryData {
    pub agent_id: String,
    #[serde(default)]
    pub session_date: Option<String>,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_history_limit() -> i64 {
    50
}

/// Input for searchforgehistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSearchForgeHistoryData {
    pub agent_id: String,
    pub query: String,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
}

// ---- Forge Import command data types ----

/// Input for importforgefromclaw
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandImportForgeFromClawData {
    pub workspace_path: String,
    pub agent_name: String,
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_message_command_roundtrip() {
        let msg = RpcMessage {
            command: "getmeta".to_string(),
            reqid: "req-123".to_string(),
            timeout: 5000,
            data: Some(serde_json::json!({"oref": "block:abc-123"})),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.command, "getmeta");
        assert_eq!(parsed.reqid, "req-123");
        assert_eq!(parsed.timeout, 5000);
        assert!(parsed.data.is_some());
    }

    #[test]
    fn test_rpc_message_response_roundtrip() {
        let msg = RpcMessage {
            reqid: "req-123".to_string(),
            resid: "res-456".to_string(),
            data: Some(serde_json::json!({"view": "term"})),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.reqid, "req-123");
        assert_eq!(parsed.resid, "res-456");
    }

    #[test]
    fn test_rpc_message_empty_fields_omitted() {
        let msg = RpcMessage {
            command: "test".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("reqid"));
        assert!(!json.contains("resid"));
        assert!(!json.contains("timeout"));
        assert!(!json.contains("cont"));
        assert!(!json.contains("cancel"));
    }

    #[test]
    fn test_rpc_message_validate_command() {
        let msg = RpcMessage {
            command: "getmeta".to_string(),
            ..Default::default()
        };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_rpc_message_validate_cancel() {
        let msg = RpcMessage {
            cancel: true,
            reqid: "req-1".to_string(),
            ..Default::default()
        };
        assert!(msg.validate().is_ok());

        // cancel without reqid or resid
        let bad = RpcMessage {
            cancel: true,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn test_rpc_message_validate_empty() {
        let msg = RpcMessage::default();
        assert!(msg.validate().is_err());
    }

    #[test]
    fn test_rpc_message_validate_both_ids() {
        let msg = RpcMessage {
            reqid: "a".to_string(),
            resid: "b".to_string(),
            ..Default::default()
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn test_command_get_meta_data() {
        let data = CommandGetMetaData {
            oref: ORef::new("block", "550e8400-e29b-41d4-a716-446655440000"),
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: CommandGetMetaData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.oref.otype, "block");
    }

    #[test]
    fn test_command_set_meta_data() {
        let mut meta = MetaMapType::new();
        meta.insert("view".into(), serde_json::json!("term"));

        let data = CommandSetMetaData {
            oref: ORef::new("block", "550e8400-e29b-41d4-a716-446655440000"),
            meta,
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: CommandSetMetaData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.meta["view"], "term");
    }

    #[test]
    fn test_wire_compat_go_rpc_message() {
        // Simulated Go-produced JSON
        let go_json = r#"{"command":"getmeta","reqid":"abc","timeout":5000,"data":{"oref":"block:123"}}"#;
        let msg: RpcMessage = serde_json::from_str(go_json).unwrap();
        assert_eq!(msg.command, "getmeta");
        assert_eq!(msg.reqid, "abc");
        assert_eq!(msg.timeout, 5000);
    }

    #[test]
    fn test_rpc_context_roundtrip() {
        let ctx = RpcContext {
            client_type: "connserver".to_string(),
            blockid: "blk-1".to_string(),
            tabid: "tab-1".to_string(),
            conn: "local".to_string(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains(r#""ctype":"connserver""#));
        let parsed: RpcContext = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.client_type, "connserver");
    }

    #[test]
    fn test_all_command_constants_non_empty() {
        // Verify all command constants are non-empty strings
        let commands = [
            COMMAND_AUTHENTICATE,
            COMMAND_AUTHENTICATE_TOKEN,
            COMMAND_DISPOSE,
            COMMAND_ROUTE_ANNOUNCE,
            COMMAND_ROUTE_UNANNOUNCE,
            COMMAND_MESSAGE,
            COMMAND_GET_META,
            COMMAND_SET_META,
            COMMAND_SET_VIEW,
            COMMAND_CONTROLLER_INPUT,
            COMMAND_CONTROLLER_STOP,
            COMMAND_CONTROLLER_RESYNC,
            COMMAND_CREATE_BLOCK,
            COMMAND_DELETE_BLOCK,
            COMMAND_FILE_READ,
            COMMAND_FILE_WRITE,
            COMMAND_FILE_APPEND,
            COMMAND_EVENT_PUBLISH,
            COMMAND_EVENT_SUB,
            COMMAND_EVENT_UNSUB,
            COMMAND_CONN_CONNECT,
            COMMAND_CONN_DISCONNECT,
            COMMAND_WORKSPACE_LIST,
            COMMAND_FOCUS_WINDOW,
            COMMAND_AI_SEND_MESSAGE,
        ];
        for cmd in &commands {
            assert!(!cmd.is_empty(), "command constant should not be empty");
        }
    }

    #[test]
    fn test_file_info_roundtrip() {
        let info = FileInfo {
            path: "/home/user/test.txt".to_string(),
            name: "test.txt".to_string(),
            size: 1024,
            isdir: false,
            mimetype: "text/plain".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: FileInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.path, "/home/user/test.txt");
        assert_eq!(parsed.size, 1024);
    }

    #[test]
    fn test_conn_status_roundtrip() {
        let status = ConnStatus {
            status: "connected".to_string(),
            connection: "ssh:myhost".to_string(),
            connected: true,
            wshenabled: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: ConnStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, "connected");
        assert!(parsed.connected);
    }

    #[test]
    fn test_wave_info_data_roundtrip() {
        let info = WaveInfoData {
            version: "0.12.15".to_string(),
            clientid: "client-123".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: WaveInfoData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, "0.12.15");
    }

    #[test]
    fn test_create_block_data_wire_compat() {
        let go_json = r#"{"tabid":"tab-1","blockdef":{"view":"term"},"magnified":true}"#;
        let parsed: CommandCreateBlockData = serde_json::from_str(go_json).unwrap();
        assert_eq!(parsed.tabid, "tab-1");
        assert!(parsed.magnified);
        assert!(parsed.blockdef.is_some());
    }
}
