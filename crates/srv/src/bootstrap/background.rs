// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of bootstrap.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;

/// Output of [`spawn_background_subsystems`] — event/watcher infrastructure
/// that needs no `AppState` and can start before it is built.
pub struct BackgroundSubsystems {
    pub event_bus: Arc<EventBus>,
    pub broker: Arc<Broker>,
    pub editor_file_watcher: Arc<backend::editor_file_watcher::EditorFileWatcher>,
    pub media_file_watcher: Arc<backend::media_file_watcher::MediaFileWatcher>,
    pub files_watcher: Arc<backend::files_watcher::FilesWatcher>,
    pub fs_watch_pool: Arc<backend::fs_watch::FsWatchPool>,
    pub config_watcher: Arc<wconfig::ConfigState>,
    pub reactive_handler: &'static reactive::ReactiveHandler,
    pub poller: Arc<Poller>,
    pub messagebus: Arc<backend::messagebus::MessageBus>,
    pub subagent_watcher: Arc<backend::subagent_watcher::SubagentWatcher>,
    pub history_service: Arc<backend::history::HistoryService>,
}

/// Event infrastructure + all background task spawns that don't need
/// `AppState` to exist yet: event bus/broker, editor file watcher, config
/// watcher, sysinfo/watchdog/activity loops, the reactive handler + poller,
/// the cloud push subscriber, messaging bridges (Discord/Telegram/Slack/
/// WhatsApp), docsite dir, messagebus, subagent watcher, history service,
/// and the session archiver.
pub fn spawn_background_subsystems(
    mstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
) -> BackgroundSubsystems {
    // Event infrastructure
    let event_bus = Arc::new(EventBus::new());
    let broker = Arc::new(Broker::new());

    // Bridge MPS events to WebSocket clients via EventBus
    let bridge = backend::eventbus::EventBusBridge::new(event_bus.clone());
    broker.set_client(Box::new(bridge));

    // Shared filesystem-watcher framework — constructed once, before any
    // consumer, all three of which now build on it (see
    // SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md §5's migration order:
    // config_watcher_fs first, editor+media here, native memory as a future
    // consumer).
    let fs_watch_pool = backend::fs_watch::FsWatchPool::new();

    // Watches files open in editor/preview panes, publishing a per-block
    // wake signal on external changes. See SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    let editor_file_watcher = backend::editor_file_watcher::EditorFileWatcher::new(fs_watch_pool.clone(), broker.clone());

    // Watches directories a Media pane is pointed at, publishing a per-block
    // wake signal when a matching-extension file changes. See
    // SPEC_MEDIA_PANE_2026_07_26.md.
    let media_file_watcher = backend::media_file_watcher::MediaFileWatcher::new(fs_watch_pool.clone(), broker.clone());

    // Watches folders open in Files panes (`fs.watch`), publishing a
    // per-block "re-list this folder" signal. See
    // SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3.
    let files_watcher = backend::files_watcher::FilesWatcher::new(fs_watch_pool.clone(), broker.clone());

    // Deploy shell integration scripts (muxlog.mjs/muxspect.mjs + rcfiles)
    // unconditionally at startup, not opportunistically from a specific
    // controller's spawn path. Previously the only call site was
    // `ShellController::start`'s interactive (empty-command) branch
    // (`blockcontroller/shell/lifecycle.rs`) — a user whose first-ever pane
    // in a fresh data dir is an Agent pane (persistent/subprocess/acp, which
    // never hits that branch) would never get the scripts deployed at all,
    // so even muxspect's own documented `node ~/.agentmux/shell/muxspect.mjs`
    // direct-path fallback would ENOENT (codex P1 on PR #2380 — a latent gap
    // in muxlog's identical deployment mechanism that PR newly depends on).
    // `deploy_scripts` is idempotent (skips if its version marker already
    // matches), so calling it here in addition to the existing call site is
    // safe, not a double-write race.
    // Two roots, deliberately:
    //   * the per-instance root the shells actually source (isolation, I6), and
    //   * the stable `~/.agentmux/shell` path docs/MUXSPECT.md and docs/MUXSH.md
    //     tell users to invoke directly, which must stay predictable.
    backend::shellintegration::deploy_scripts(&backend::shellintegration::integration_base());
    backend::shellintegration::deploy_scripts(&backend::shellintegration::documented_shell_root());

    // Config watcher (created before sysinfo loop so it can read telemetry:interval)
    let config_watcher = Arc::new(wconfig::ConfigState::with_config(wconfig::build_default_config()));

    // Load user's settings.json from disk (merges with defaults)
    backend::config_watcher_fs::load_settings_from_disk(&config_watcher);

    // Watch settings.json for changes and broadcast to WebSocket clients
    backend::config_watcher_fs::spawn_settings_watcher(
        fs_watch_pool.clone(),
        config_watcher.clone(),
        event_bus.clone(),
    );

    // The user's own widgets.json beside settings.json, merged over the
    // built-in widgets (Pane Tab contract Phase 6: where an `ext:` widget is
    // added). Same load-then-watch shape, same pool.
    backend::user_widgets::load_user_widgets_from_disk(&config_watcher);
    backend::user_widgets::spawn_user_widgets_watcher(
        fs_watch_pool.clone(),
        config_watcher.clone(),
        event_bus.clone(),
    );

    // Browser pane start page — same load-then-watch shape as settings.json
    // above, on the SAME fs_watch_pool instance, so it rides the existing
    // GetFullConfig/live-broadcast pipeline instead of a parallel one. See
    // docs/specs/SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md §3.2.
    backend::browser_start_page::load_start_page_from_disk(&config_watcher);
    backend::browser_start_page::spawn_start_page_watcher(
        fs_watch_pool.clone(),
        config_watcher.clone(),
        event_bus.clone(),
    );

    // Start sysinfo collection loop (interval configurable via telemetry:interval)
    let sysinfo_broker = broker.clone();
    let sysinfo_config = config_watcher.clone();
    tokio::spawn(async move {
        sysinfo::run_sysinfo_loop(sysinfo_broker, sysinfo_config, "local".to_string()).await;
    });

    // Start agent process watchdog (kills panes that exceed max-runtime or idle-output limits)
    let watchdog_config = config_watcher.clone();
    tokio::spawn(async move {
        backend::blockcontroller::watchdog::run_watchdog_loop(watchdog_config).await;
    });

    // Recover a missing session title for a running agent (the Swarm row and
    // pane header read it); see activity_watcher.rs —
    // reads reactive::get_global_handler() as its registry, so it needs no
    // AppState and can start before AppState is built (matches sysinfo/watchdog above).
    let activity_mstore = Arc::clone(mstore);
    let activity_filestore = Arc::clone(filestore);
    let activity_event_bus = event_bus.clone();
    // Every ambient call's spend goes to the status bar's totals (ambient/spend.rs).
    crate::ambient::spend::publish_on(event_bus.clone());
    tokio::spawn(async move {
        backend::reactive::activity_watcher::run_agent_summary_loop(
            activity_mstore, activity_filestore, activity_event_bus,
        ).await;
    });

    // Push each agent's todo checklist + in-flight tool (swarm rows under the
    // agent name). Separate loop from the summary above deliberately: this one
    // is a tail-read + JSON parse with no model call, so it sweeps every 3s
    // instead of 20s — a checklist that lags is worse than none. Same
    // registry, same no-AppState property.
    let progress_filestore = Arc::clone(filestore);
    let progress_broker = broker.clone();
    tokio::spawn(async move {
        backend::reactive::progress_watcher::run_agent_progress_loop(
            progress_filestore, progress_broker,
        ).await;
    });

    // Reactive handler (global singleton) + poller
    let reactive_handler = reactive::get_global_handler();
    reactive_handler.set_input_sender(Arc::new(|block_id: &str, data: &[u8]| {
        backend::blockcontroller::send_input(
            block_id,
            backend::blockcontroller::BlockInputUnion::data(data.to_vec()),
            None,
        )
    }));
    // Controller-aware delivery (SPEC_AGENT_CONTROL_PROTOCOL §6 / Phase 3): persistent
    // stream-json and ACP agents have no PTY, so muxbus Tier-1 keystroke injection
    // silently misses them. Route those through their structured channel (live stdin /
    // session/prompt) — which also steers the agent mid-turn — and fall back to PTY
    // keystrokes only for terminal-based agents.
    reactive_handler.set_message_sender(Arc::new(|block_id: &str, message: &str| {
        match backend::blockcontroller::deliver_agent_message(block_id, message) {
            Ok(backend::blockcontroller::AgentDelivery::Structured) => Ok(reactive::SenderDelivery::Delivered),
            Ok(backend::blockcontroller::AgentDelivery::StructuredDeferred) => Ok(reactive::SenderDelivery::Deferred),
            Ok(backend::blockcontroller::AgentDelivery::Pty) => Ok(reactive::SenderDelivery::Pty),
            Err(e) => Err(e),
        }
    }));
    // Recipient-identity check (issue #2695): before delivering, compare the
    // resolved target's own live, spawn-time-captured identity (queried from
    // the actual controller, independent of reactive_handler's own
    // name/UID-to-block maps) against who the jekt was addressed to. See
    // Controller::agent_id's doc comment for why this can't be derived from
    // reactive_handler's own state.
    reactive_handler.set_agent_identity_confirmer(Arc::new(|block_id: &str| {
        backend::blockcontroller::get_controller(block_id).and_then(|c| c.agent_id())
    }));
    // Stable counterpart to the above (`AGENTMUX_AGENT_ID`, frozen at spawn)
    // — see `Controller::stable_agent_id`'s and `NameBindings::stable`'s
    // doc comments for why a jekt tagged with the stable ID needs its own,
    // independently-sourced confirmer instead of reusing the live one.
    reactive_handler.set_stable_agent_identity_confirmer(Arc::new(|block_id: &str| {
        backend::blockcontroller::get_controller(block_id).and_then(|c| c.stable_agent_id())
    }));
    // Identity M2: the UID confirmer, consulted only for targets that
    // resolved by UID — see `Handler::uid_identity_confirmer` for why it is
    // a separate check and why `None` must mean "unverifiable".
    reactive_handler.set_uid_identity_confirmer(Arc::new(|block_id: &str| {
        backend::blockcontroller::get_controller(block_id).and_then(|c| c.stable_agent_uid())
    }));
    // Identity M2: liveness — the garbage collector that replaces
    // eviction-by-name (spec §4.4.4 Q9). A block with no controller is dead
    // and is swept when a name it held is resolved.
    reactive_handler.set_block_liveness(Arc::new(|block_id: &str| {
        backend::blockcontroller::get_controller(block_id).is_some()
    }));
    let poller = Arc::new(Poller::new(
        PollerConfig {
            muxbus_url: None,
            muxbus_token: None,
            poll_interval_secs: reactive::DEFAULT_POLL_INTERVAL_SECS,
        },
        reactive_handler,
    ));

    // Cloud delivery's state for the UI, the sign-in notification and the
    // agents' pause/resume notes. Reads nothing from the keychain itself.
    crate::muxbus::delivery_status::install(event_bus.clone(), broker.clone());
    // Cloud push subscriber — single WS connection per sidecar that the cloud
    // uses to push reactive injections instead of polling. The WS connection
    // itself is a no-op until the user connects via muxbus.login, but
    // `init_global` registers the muxbus credential with the broker
    // scheduler unconditionally, which calls `Store::muxbus_is_fresh()` —
    // a real, synchronous OS-keychain read — almost immediately. That read
    // is the automatic (non-user-triggered) keychain touch
    // docs/retro/retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md
    // investigates; skipping `init_global` entirely is not a no-op the way
    // the comment above might suggest.
    //
    // `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER` exists so `crates/srv/tests/
    // integration_test.rs` can spawn the real binary without that read
    // firing. Those tests spawn an ad-hoc/dev-signed `target/debug`
    // binary, which is never on the Keychain ACL's trusted-signature list
    // (only specific notarized, Developer-ID-signed installs are) — so
    // every spawned test process gets its own interactive consent prompt
    // for the SAME real credential, on whatever macOS account happens to
    // be running the test. Confirmed live: a `cargo test -p agentmux-srv`
    // run produced 4 near-simultaneous prompts, one per spawned subprocess,
    // on a shared dev machine. Off by default — the shipped app and `task
    // dev` must keep muxbus reconnect-on-launch working exactly as before.
    //
    // `isolated_muxbus_reconnect_enabled()` closes a second, broader gap
    // the env-var flag above doesn't (see
    // docs/retro/retro-macos-0560-stale-cef-cache-launch-crash-2026-09-16.md):
    // every local `task package`/`task package:macos`/`task package:linux`
    // build bakes a brand-new, randomized per-build channel (and, on
    // macOS, a bundle identifier derived from it) into the binary, so it
    // is a never-before-seen code signature to the Keychain every single
    // time — the SAME already-`Always Allow`'d muxbus credential prompts
    // again on every fresh local build, indefinitely. Unlike
    // `isolated_auth_enabled`/`isolated_settings_enabled` (which also
    // isolate `task dev`'s `dev-<branch>` channel), this flag deliberately
    // exempts `dev-*` — see its own doc comment for why a per-branch-stable
    // dev channel doesn't have the per-build-random-identity problem this
    // one exists to solve. Skipping this eager call is also NOT permanent
    // for the process's lifetime on an isolated channel: `muxbus.login`
    // lazily initializes the subscriber the moment the user explicitly
    // logs in (see `muxbus_handlers.rs`), and `muxbus.status` gates on
    // subscriber presence rather than re-deriving this same channel check,
    // so it starts working immediately after that login with no restart
    // required.
    let muxbus_reason = agentmux_common::isolated_muxbus_reconnect_reason();
    if cloud_subscriber_disabled_from_env() {
        tracing::info!("cloud_subscriber: init_global skipped (AGENTMUX_DISABLE_CLOUD_SUBSCRIBER)");
    } else if muxbus_reason.is_isolated() {
        tracing::info!(
            reason = muxbus_reason.as_str(),
            "cloud_subscriber: init_global skipped (ISOLATED — channel-scoped, no automatic MuxBus reconnect)"
        );
    } else {
        crate::muxbus::cloud_subscriber::CloudSubscriber::init_global(id_store.clone());
    }
    // W3-S D1b: publish each agent's instance-certified WAN key to the
    // account's cloud directory. Not gated on the subscriber: a logged-out
    // or isolated channel simply finds no token and publishes nothing.
    crate::muxbus::wan_publish::spawn(mstore.clone(), id_store.clone());

    // Discord messaging bridge — connects to Discord Gateway if configured.
    // Set messaging:discord:enabled + messaging:discord:token in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_discord_enabled {
            match settings.messaging_discord_token.clone() {
                Some(token) if !token.is_empty() => {
                    messaging::discord::DiscordBridge::init_global(
                        messaging::discord::DiscordConfig {
                            token,
                            channel_id: settings.messaging_discord_channel.clone(),
                            target_agent: settings.messaging_discord_target.clone(),
                            guild_id: settings.messaging_discord_guild.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "discord bridge: enabled but messaging:discord:token is not set in settings.json"
                    );
                }
            }
        }
    }

    // Telegram messaging bridge — long-polls getUpdates if configured.
    // Set messaging:telegram:enabled + messaging:telegram:token in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_telegram_enabled {
            match settings.messaging_telegram_token.clone() {
                Some(token) if !token.is_empty() => {
                    let allowed_chat_ids = settings
                        .messaging_telegram_allowed_chats
                        .split(',')
                        .filter_map(|s| s.trim().parse::<i64>().ok())
                        .collect::<Vec<_>>();
                    let default_chat_id = settings
                        .messaging_telegram_default_chat
                        .as_deref()
                        .and_then(|s| s.parse::<i64>().ok());
                    messaging::telegram::TelegramBridge::init_global(
                        messaging::telegram::TelegramConfig {
                            token,
                            allowed_chat_ids,
                            default_chat_id,
                            target_agent: settings.messaging_telegram_target.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "telegram bridge: enabled but messaging:telegram:token is not set in settings.json"
                    );
                }
            }
        }
    }

    // Slack messaging bridge — opens a Socket Mode connection if configured.
    // Set messaging:slack:enabled + messaging:slack:bot_token + messaging:slack:app_token
    // in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_slack_enabled {
            match (
                settings.messaging_slack_bot_token.clone(),
                settings.messaging_slack_app_token.clone(),
            ) {
                (Some(bot_token), Some(app_token))
                    if !bot_token.is_empty() && !app_token.is_empty() =>
                {
                    messaging::slack::SlackBridge::init_global(
                        messaging::slack::SlackConfig {
                            bot_token,
                            app_token,
                            channel_id: settings.messaging_slack_channel.clone(),
                            target_agent: settings.messaging_slack_target.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "slack bridge: enabled but messaging:slack:bot_token and/or \
                         messaging:slack:app_token is not set in settings.json"
                    );
                }
            }
        }
    }

    // WhatsApp Cloud API messaging bridge — outbound send + inbound webhook
    // receiver. Unlike Discord/Telegram/Slack there is no bridge-managed
    // network connection to open at startup: inbound delivery is passive
    // HTTP (the GET/POST /webhook/whatsapp routes, registered unauthenticated
    // in server/mod.rs — Meta cannot supply X-AuthKey), reachable only once
    // the operator's own tunnel is up and the callback URL is registered in
    // Meta's App Dashboard, both manual one-time steps this process does not
    // perform (v1 does not manage a tunnel subprocess — see
    // messaging/whatsapp/mod.rs). Set messaging:whatsapp:enabled +
    // access_token/app_secret/webhook_verify_token in settings.json to activate.
    // See docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_whatsapp_enabled {
            match (
                settings.messaging_whatsapp_access_token.clone(),
                settings.messaging_whatsapp_app_secret.clone(),
                settings.messaging_whatsapp_webhook_verify_token.clone(),
            ) {
                (Some(token), Some(secret), Some(verify_token))
                    if !token.is_empty() && !secret.is_empty() && !verify_token.is_empty() =>
                {
                    messaging::whatsapp::WhatsAppBridge::init_global(
                        messaging::whatsapp::WhatsAppConfig {
                            phone_number_id: settings.messaging_whatsapp_phone_number_id.clone(),
                            access_token: token,
                            app_secret: secret,
                            webhook_verify_token: verify_token,
                            target_agent: settings.messaging_whatsapp_target.clone(),
                            fallback_template: settings.messaging_whatsapp_fallback_template.clone(),
                            fallback_template_lang: settings
                                .messaging_whatsapp_fallback_template_lang
                                .clone()
                                .unwrap_or_else(|| "en_US".to_string()),
                        },
                        reqwest::Client::new(),
                    );
                    if settings.messaging_whatsapp_tunnel_domain.is_empty() {
                        tracing::warn!(
                            "whatsapp bridge: enabled but messaging:whatsapp:tunnel_domain is not set — \
                             point your own tunnel at this instance's webhook port and register \
                             https://<your-domain>/webhook/whatsapp in Meta App Dashboard > WhatsApp > Configuration"
                        );
                    } else {
                        tracing::info!(
                            "whatsapp bridge: initialized — webhook callback = https://{}/webhook/whatsapp \
                             — verify this is registered in Meta App Dashboard > WhatsApp > Configuration \
                             (v1 does not manage the tunnel subprocess; ensure it's already running)",
                            settings.messaging_whatsapp_tunnel_domain
                        );
                    }
                }
                _ => {
                    tracing::warn!(
                        "whatsapp bridge: enabled but one of messaging:whatsapp:{{access_token,app_secret,webhook_verify_token}} is not set in settings.json"
                    );
                }
            }
        }
    }

    // Set up docsite directory
    if let Some(app_path) = base::get_mux_app_path() {
        let docsite_dir = app_path.join("docsite");
        docsite::set_docsite_dir(docsite_dir);
    }

    // Local MessageBus for inter-agent communication
    let messagebus = Arc::new(backend::messagebus::MessageBus::new());

    // Subagent watcher — monitors Claude Code session dirs for spawned subagents
    let subagent_watcher = backend::subagent_watcher::SubagentWatcher::spawn(
        event_bus.clone(),
        mstore.clone(),
        id_store.clone(),
        identity_store.clone(),
    );
    // Wires `subagent:backfill_status` (scoped, persisted MPS event) — see
    // `SubagentWatcher`'s `broker` field doc comment for why this is a
    // post-construction setter rather than a constructor parameter.
    subagent_watcher.set_broker(broker.clone());
    // Registered as a global so blockcontroller/persistent/stdout_reader.rs's turn-end
    // reconciliation hook (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_
    // 2026_07_20 Phase A) can reach it without threading an Arc through
    // PersistentSubprocessController::new and every one of its callers.
    backend::subagent_watcher::set_global(subagent_watcher.clone());

    // History service — discovers and indexes past CLI agent conversations
    let history_service = Arc::new(backend::history::HistoryService::new());
    history_service.warm_in_background();

    // Install the pinned CLI of every provider the user's own agents use, in
    // the background, so the first open after a pin bump doesn't wait on npm
    // (SPEC_AGENT_OPEN_LATENCY_2026_09_27.md §4.1). Templates don't count:
    // they span every provider. AGENTMUX_NO_CLI_WARM=1 turns it off.
    if std::env::var("AGENTMUX_NO_CLI_WARM").map_or(true, |v| v != "1") {
        match (
            agentmux_common::DataPaths::from_env(),
            mstore.agent_def_list(),
        ) {
            (Some(paths), Ok(defs)) => {
                let providers: Vec<String> = defs
                    .into_iter()
                    .filter(|d| d.is_seeded != 1)
                    .map(|d| d.provider)
                    .collect();
                backend::cli_install::warm_used_providers(paths, providers);
            }
            (_, Err(e)) => {
                tracing::warn!(error = %e, "cli warm-up: cannot list agent definitions; skipped")
            }
            (None, _) => tracing::warn!("cli warm-up: no data paths; skipped"),
        }
    }

    // Session archiver — auto-archive sessions inactive for >7 days, cap at 2 GB.
    // Skip if home directory can't be determined (would otherwise fall back to a
    // relative path and create archives under the current working directory).
    if let Some(archive_dir) = backend::session_archive::default_archive_dir() {
        let archiver = Arc::new(backend::session_archive::SessionArchiver::new(
            mstore.clone(),
            filestore.clone(),
            7,                              // inactive days
            2 * 1024 * 1024 * 1024,         // 2 GB max
            archive_dir,
        ));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                match archiver.sweep().await {
                    Ok(stats) => tracing::info!(?stats, "session archiver sweep complete"),
                    Err(e) => tracing::warn!(error = %e, "session archiver sweep failed"),
                }
            }
        });
    } else {
        tracing::warn!("session archiver: home dir unavailable, archiver disabled");
    }

    BackgroundSubsystems {
        event_bus,
        broker,
        editor_file_watcher,
        media_file_watcher,
        files_watcher,
        fs_watch_pool,
        config_watcher,
        reactive_handler,
        poller,
        messagebus,
        subagent_watcher,
        history_service,
    }
}


/// Should `CloudSubscriber::init_global` be skipped this run?
///
/// Presence-based, matching the `AGENTMUX_DEV`/`AGENTMUX_TRAY` idiom
/// elsewhere in this codebase. See the call site's doc comment for why
/// skipping this matters: `init_global` triggers a real, synchronous
/// OS-keychain read almost immediately, which macOS gates behind an
/// interactive consent prompt for any process whose code signature isn't
/// on the target credential's trusted-signer ACL — exactly the case for an
/// ad-hoc/dev-signed `target/debug` binary spawned by an integration test.
fn cloud_subscriber_disabled_from_env() -> bool {
    std::env::var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER").is_ok()
}

#[cfg(test)]
mod cloud_subscriber_gate_tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize: both tests mutate the same process-global env var, and
    // `cargo test` runs tests in parallel by default within one binary.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn defaults_to_enabled_so_shipped_and_dev_behavior_is_unchanged() {
        let _guard = lock();
        std::env::remove_var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER");
        assert!(
            !cloud_subscriber_disabled_from_env(),
            "must default to false — the shipped app and `task dev` need the \
             real muxbus reconnect-on-launch behavior with no opt-out set"
        );
    }

    #[test]
    fn is_disabled_when_the_var_is_present_regardless_of_value() {
        let _guard = lock();
        // Presence-based, not value-based — matches AGENTMUX_DEV/AGENTMUX_TRAY.
        // An empty value must still disable it; a caller setting the var to
        // "" to mean "off" would otherwise silently get the opposite of
        // what every other flag in this idiom does.
        std::env::set_var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "");
        assert!(cloud_subscriber_disabled_from_env());
        std::env::set_var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "1");
        assert!(cloud_subscriber_disabled_from_env());
        std::env::remove_var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER");
    }
}
