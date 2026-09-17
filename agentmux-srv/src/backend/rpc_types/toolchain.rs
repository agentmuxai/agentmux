// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the Toolchain modal and the widget HTTP proxies.
//!
//! The toolchain commands built their responses from inline `json!({..})`; the
//! widget ones do too, and additionally read their REQUESTS straight off a
//! `serde_json::Value` rather than deserializing a struct. See
//! `WidgetHealthResult` for why that request half stays untyped.

use serde::{Deserialize, Serialize};

/// Request for `toolchain.env`, which ignores its payload.
///
/// Registered as `Option<Self>`: the stub sends `{}` and a client that omits
/// `data` sends `null`, and serde accepts each of those from only one of `()`
/// and a struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ToolchainEnvReq {}

/// Response for `toolchain.env` — the environment srv resolves tools in.
/// Powers the Toolchain modal's Environment section, so PATH problems are
/// diagnosable without shelling into the app.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ToolchainEnvResult {
    pub path: String,
    /// camelCase on the wire, unlike every other field in this module. It was
    /// written by hand as `pathSource` and the frontend reads that, so the
    /// rename preserves it rather than "fixing" a live key.
    ///
    /// Needs BOTH attributes: ts-rs does not read `serde(rename)`, so without
    /// the `ts` one the generated binding would say `path_source` — a key
    /// nothing sends and nothing reads.
    #[serde(rename = "pathSource")]
    #[ts(rename = "pathSource")]
    pub path_source: String,
    pub os: String,
    pub arch: String,
}

/// One entry in `toolchain.versions`' request.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ToolchainPackage {
    /// Caller-chosen key. The response is keyed by this, not by `package`, so
    /// a caller asking about the same npm package under two ids gets two
    /// answers.
    pub id: String,
    /// npm package name.
    pub package: String,
}

/// Request for `toolchain.versions`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ToolchainVersionsReq {
    /// An unparseable entry drops out silently rather than failing the whole
    /// request, which is why this is `serde(default)` and why the handler
    /// filters rather than erroring: one bad package name should not cost the
    /// modal every other version it asked for.
    #[serde(default)]
    pub packages: Vec<ToolchainPackage>,
}

/// Response for `widget.health`.
///
/// `healthy: false` is a normal answer, not an error: connection-refused and
/// timeout both land here rather than failing the RPC, because "is this widget
/// up" has a legitimate negative answer.
///
/// The REQUEST stays a `serde_json::Value` read field-by-field. That is not an
/// oversight: the handler clamps a bad port and a bad path into
/// `healthy: false` answers instead of rejecting them, and a typed request
/// would turn those into deserialize errors. Registering `Value` as the request
/// type keeps that tolerance exactly while still recording this response type —
/// which `register_handler` could not do.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct WidgetHealthResult {
    pub healthy: bool,
    /// Null when the exchange never produced one (refused, timed out, or the
    /// port/path was rejected before any request was made).
    #[ts(type = "number | null")]
    pub status_code: Option<u16>,
}

/// Response for `widget.api`.
///
/// `ok` is about the HTTP exchange completing, NOT about the status code: a
/// 500 is `ok: true` with `status_code: 500`. `ok: false` means transport
/// failure or a rejected port/path, and then `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct WidgetApiResult {
    pub ok: bool,
    #[ts(type = "number | null")]
    pub status_code: Option<u16>,
    #[ts(type = "string | null")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}
