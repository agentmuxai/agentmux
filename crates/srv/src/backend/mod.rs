// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


pub mod agent_admission;
pub mod agent_grants;
pub mod agent_line;
pub mod agent_seat;
pub mod agent_availability;
pub mod agent_status;
pub mod agent_state;
pub mod attachments;
pub mod agent_color;
pub mod palette;
pub mod account_availability;
pub mod agent_env;
pub mod endpoint_notice;
pub mod agent_runtime_state;
pub mod install_events;
pub mod signin_link_event;
pub mod agent_config;
pub mod agent_skill_files;
pub mod agent_registry_lookup;
pub mod agent_resolve;
pub mod name_resolution;
pub mod identity_spawn;
pub mod agent_session;
pub mod blockcontroller;
pub mod bash_background;
pub mod claudeai_connectors;
pub mod cli_messaging;
pub mod cli_native;
pub mod bookmarks_store;
pub mod browser_profiles_store;
pub mod layout_file;
pub mod browser_start_page;
pub mod user_widgets;
pub mod user_attention;
pub mod widget;
pub mod workspace_copy;
pub mod job_templates;
pub mod vault;
pub mod build_cache;
pub mod build_cache_service;
pub mod disk_notice;
pub mod old_versions;
pub mod old_versions_service;
pub mod bundle_export;
pub mod bundle_import;
pub mod bundle_import_skill_text;
pub mod bundle_use;
pub mod bundle_widgets;
pub mod bundle_validate;
pub mod background_task_feed;
pub mod desktop_gate;
pub mod dock_snapshot;
pub mod fleet_feed;
pub mod fleet_source;
pub mod narrated_events;
pub mod pending_background_pids;
pub mod mcp_config_file;
pub mod mcp_health;
pub mod mcp_health_meta;
pub mod message_origin;
pub mod mcp_json_identity;
pub mod mcp_launch;
pub mod mcp_observed;
pub mod mcp_probe;
pub mod mcp_rewrite;
pub mod mcp_selftest;
pub mod mcp_seed;
/// Phase E.4.B Phase 4 — pure layout-tree helpers (Rust port of layoutTree.ts).
pub mod layout;
pub mod project_instructions;
pub mod agent_runtime;
pub mod agent_context_usage;
pub mod agent_api_retry;
pub mod providers;
pub mod cli_download;
pub mod cli_install;
pub mod cli_prune;
pub mod cli_uninstall;
pub mod cli_uv;
pub mod cli_notice;
pub mod startup_files;
pub mod confined_read;
pub mod workdir_fs;
pub mod model_catalog;
pub mod codex_models;
pub mod config_watcher_fs;
pub mod editor_file_watcher;
pub mod editor_probe;
pub mod file_icons;
pub mod files_watcher;
pub mod fs_ops;
pub mod fs_watch;
pub mod media_file_watcher;
pub mod ijson;
pub mod docsite;
pub mod eventbus;
pub mod agent_seed;
pub mod operator_config_seed;
pub mod history;
pub mod slug_migration;
pub mod host_os;
pub mod lan;
pub mod lsp;
pub mod messagebus;
pub mod notify;
pub mod oref;
pub mod process_identity;
pub mod process_tracker;
pub mod reactive;
pub mod remote;
pub mod registry_reconcile;
pub mod resume_preflight;
pub mod rpc;
pub mod rpc_types;
pub mod schema;
pub mod service;
pub mod session_archive;
pub mod history_changed;
pub mod history_registry;
pub mod history_usage;
pub mod session_backfill;
pub mod claude_layout;
pub mod context_reading;
pub mod continuity;
pub mod continuity_relocate;
pub mod continuity_segments;
pub mod audit_activity;
pub mod audit_live;
pub mod audit_query;
pub mod memory;
pub mod agent_bundle_sidecar;
pub mod continuity_state;
/// Skills that ship with AgentMux, seeded into the catalog once.
pub mod builtin_skills;
/// Reading a skill from a folder, a git repository or a URL, for import.
pub mod computer_use_skill;
pub mod skill_catalog;
pub mod skill_file;
pub mod skill_import;
pub mod skill_slugs;
/// What of Personal Memory fits in a delivery, and what the agent is told about the rest.
pub mod shellexec;
pub mod transcript;
pub mod transcript_backfill;
pub mod blocking;
pub mod gh_guard;
pub mod publish_guard;
pub mod account_login_guard;
pub mod pane_env;
pub mod posix_shell;
pub mod shellintegration;
pub mod shells;
pub mod swarm_remote;
pub mod sampler;
// The commit attribution that uses it is Windows-only; the logic is tested everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod own_instance;
pub mod stall_watch;
pub mod net_counters;
pub mod sysinfo;
pub mod blockstats_detail;
pub mod tower;
pub mod tool_calls;
pub mod update;
pub mod agent_image_digest;
pub mod upgrade_notice;
pub mod background_set;
pub mod background_work;
pub mod upgrade_request;
pub mod storage;
pub mod subagent_watcher;
pub mod supervisor;
pub mod userinput;
pub mod user_stop;
pub mod viewer;
pub mod text_encoding;
pub mod base;
pub mod obj;
pub mod wconfig;
pub mod wcore;
pub mod mps;
pub mod mps_events;
pub mod osc_extractor;
pub mod tool_store;
pub(crate) mod packaged_tools;
pub mod agent_env_scope;
pub(crate) mod spawn_env;
pub mod agent_tools;
pub mod container;
pub mod runtime_adapter;
pub mod route_audience;
pub mod audience;
pub mod registry_probe;
pub mod cron;
pub mod jobs;
pub mod dev_proxy;
pub mod dev_keepalive;
pub mod dev_relay;
pub mod shell_node;
pub mod work_facts;
/// The Kanban's reminders and PR links (SPEC_WARDEN_KANBAN_2026_10_20.md).
pub mod kanban;
pub mod task_media;
/// The wake at an out-of-quota agent's reset (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §7).
pub mod limit_wake;
/// Model routes and the gateway that carries an agent's model traffic (SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md).
pub mod model_gateway;
/// Which model a Claude agent's sub-agents run on (SPEC_SUBAGENT_MODELS_2026_10_25.md).
pub mod subagent_model;
/// An agent accepting or sending back a task in Review.
pub mod kanban_delegate;
/// The `agents:always_available` setting (SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md).
pub mod always_available;
/// The operations the super agent reviews (SPEC_SUPER_AGENT_2026_10_25.md).
pub mod sensitive_ops;
/// The requests for those operations and what became of them.
pub mod sensitive_requests;
/// Replies that say nothing are not sent (SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md).
pub mod ack_policy;
/// Quota mode, a fleet setting (SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md section 5).
pub mod quota_mode;
/// How a bulletin reads to its receiver (SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md).
pub mod bulletin;
/// Holding a routine message for an agent whose session has gone cold (SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_2026_10_25.md).
pub mod cold_hold;
/// Google connectors: Calendar, Drive and Gmail for agents, called by srv.
pub mod google;
/// A leader hears when its workers end a turn without reporting, fail or stop.
pub mod route_credits;
pub mod worker_notes;

pub use oref::ORef;
