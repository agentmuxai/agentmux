// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AWS credentials for container agents, brokered per use.
//! Spec: docs/specs/SPEC_CONTAINER_AGENT_AWS_CREDENTIALS_2026_10_31.md.
//!
//! A container agent's SDK runs `agentmux-bashwrap aws-credentials --profile <grant>` (a `credential_process`), which
//! asks `POST /api/v1/aws/credentials` (`server::app_api::aws_credentials`) with the container's token. srv checks the
//! host's grants ([`policy`]), reuses or mints a session on the host ([`sessions`], [`mint`]) and answers with the
//! credentials or a sentence ([`request`]). Nothing is written into the container.

pub mod container;
pub mod mint;
pub mod policy;
pub mod request;
pub mod sessions;
pub mod tools;
