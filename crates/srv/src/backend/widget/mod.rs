// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Widgets: what a widget may reach, the catalog and packages that install it, its requests and its signature.

pub mod access;
#[cfg(unix)]
mod beneath;
pub mod catalog;
pub mod net;
pub mod packages;
pub mod requests;
pub mod signature;
