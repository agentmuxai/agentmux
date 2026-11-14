// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The messaging bridges (Discord, Telegram, Slack, WhatsApp) started once the settings are
//! loaded, one function each. Moved out of `spawn_background_subsystems` without a change in
//! behaviour.

use super::*;

/// Starts every messaging bridge the settings turn on.
pub(super) fn start_messaging_bridges(config_watcher: &Arc<wconfig::ConfigState>) {
    start_discord_bridge(config_watcher);
    start_telegram_bridge(config_watcher);
    start_slack_bridge(config_watcher);
    start_whatsapp_bridge(config_watcher);
}

/// Discord messaging bridge — connects to Discord Gateway if configured.
/// Set messaging:discord:enabled + messaging:discord:token in settings.json to activate.
fn start_discord_bridge(config_watcher: &Arc<wconfig::ConfigState>) {
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
                    crate::http_client::client(),
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

/// Telegram messaging bridge — long-polls getUpdates if configured.
/// Set messaging:telegram:enabled + messaging:telegram:token in settings.json to activate.
fn start_telegram_bridge(config_watcher: &Arc<wconfig::ConfigState>) {
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
                    crate::http_client::client(),
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

/// Slack messaging bridge — opens a Socket Mode connection if configured.
/// Set messaging:slack:enabled + messaging:slack:bot_token + messaging:slack:app_token
/// in settings.json to activate.
fn start_slack_bridge(config_watcher: &Arc<wconfig::ConfigState>) {
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
                    crate::http_client::client(),
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

/// WhatsApp Cloud API messaging bridge — outbound send + inbound webhook
/// receiver. Unlike Discord/Telegram/Slack there is no bridge-managed
/// network connection to open at startup: inbound delivery is passive
/// HTTP (the GET/POST /webhook/whatsapp routes, registered unauthenticated
/// in server/mod.rs — Meta cannot supply X-AuthKey), reachable only once
/// the operator's own tunnel is up and the callback URL is registered in
/// Meta's App Dashboard, both manual one-time steps this process does not
/// perform (v1 does not manage a tunnel subprocess — see
/// messaging/whatsapp/mod.rs). Set messaging:whatsapp:enabled +
/// access_token/app_secret/webhook_verify_token in settings.json to activate.
/// See docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md.
fn start_whatsapp_bridge(config_watcher: &Arc<wconfig::ConfigState>) {
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
                    crate::http_client::client(),
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
