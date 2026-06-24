// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Discord REST API client — message send only.
//!
//! Rate limiting: Discord returns X-RateLimit-* headers. We log 429s and
//! return an error; the caller (gateway loop) logs a warning. Full rate-limit
//! header handling is deferred to Phase 2.

use crate::messaging::{EmbedField, MsgEmbed, OutboundMsg};

use super::types::{DiscordEmbed, DiscordEmbedField, DiscordEmbedFooter, SendMessageBody};

const DISCORD_API: &str = "https://discord.com/api/v10";

/// POST /channels/{channel_id}/messages
pub async fn send_message(
    http: &reqwest::Client,
    token: &str,
    channel_id: &str,
    msg: &OutboundMsg,
) -> Result<(), String> {
    let embeds = msg.embed.as_ref().map(to_discord_embed).into_iter().collect();

    let body = SendMessageBody {
        content: msg.text.clone(),
        embeds,
    };

    let url = format!("{}/channels/{}/messages", DISCORD_API, channel_id);
    let resp = http
        .post(&url)
        .header("Authorization", format!("Bot {}", token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("discord rest: http error: {e}"))?;

    let status = resp.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("discord rest: rate limited: {body}"));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("discord rest: {status}: {body}"));
    }

    Ok(())
}

fn to_discord_embed(embed: &MsgEmbed) -> DiscordEmbed {
    DiscordEmbed {
        title: embed.title.clone(),
        description: embed.description.clone(),
        color: embed.color,
        fields: embed.fields.iter().map(to_discord_field).collect(),
        footer: embed
            .footer
            .as_ref()
            .map(|t| DiscordEmbedFooter { text: t.clone() }),
    }
}

fn to_discord_field(f: &EmbedField) -> DiscordEmbedField {
    DiscordEmbedField {
        name: f.name.clone(),
        value: f.value.clone(),
        inline: f.inline,
    }
}
