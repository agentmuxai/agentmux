// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The transcript tail reader now lives with the transcript code (`backend::transcript::tail`), so
//! the modules below the server can use it; this keeps the old path for its callers here.

pub(crate) use crate::backend::transcript::tail::*;
