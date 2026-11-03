// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The in-app updater: the check for a newer release, its download and signature, and the install, unpack and restart.

pub mod check;
pub mod download;
pub mod install;
pub mod restart;
pub mod signature;
pub mod unpack;
