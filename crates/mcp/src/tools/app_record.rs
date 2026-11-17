// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `App` with `view`: this machine's record of AgentMux's own size over weeks
//! (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820), read from srv's
//! `GET /api/v1/app/record`. `summary` says whether AgentMux has grown, `series` lists the hours or the
//! five-minute lines, `export` returns the lines as they are stored, to attach to a bug. The record holds numbers
//! and stable keys only and never leaves the computer.

use super::app_tool::{ago, gb, mb, s, str_arg};
use super::*;

const DAY_MS: i64 = 86_400_000;
/// The answer of `export` is cut at this many bytes of text.
const EXPORT_BYTES: usize = 200_000;
const MAX_HOURS: u64 = 336;

pub(super) async fn record(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let view = str_arg(arguments, "view").unwrap_or_else(|| "summary".to_string());
    let now_ms = agentmux_common::time::now_ms();
    match view.as_str() {
        "summary" => {
            let from = (now_ms - 8 * DAY_MS).max(0).to_string();
            let rolls = get(cx, &[("kind", "rollup"), ("limit", "200"), ("from_ms", &from)]).await?;
            let raw = get(cx, &[("kind", "raw"), ("limit", "1")]).await?;
            Ok(describe_summary(&rolls["inventory"], raw["lines"].get(0), rolls["lines"].as_array().map(Vec::as_slice).unwrap_or_default(), now_ms))
        }
        "series" | "export" => {
            let raw = str_arg(arguments, "kind").as_deref() == Some("raw");
            let hours = arguments.get("hours").and_then(Value::as_u64).unwrap_or(24).clamp(1, MAX_HOURS);
            let limit = if raw { (hours * 12).min(600) } else { hours }.to_string();
            let from = (now_ms - hours as i64 * 3_600_000).max(0).to_string();
            let day = str_arg(arguments, "day");
            let mut query = vec![("kind", if raw { "raw" } else { "rollup" }), ("limit", limit.as_str())];
            match day.as_deref() {
                Some(day) => query.push(("day", day)),
                None => query.push(("from_ms", from.as_str())),
            }
            let body = get(cx, &query).await?;
            let lines = body["lines"].as_array().map(Vec::as_slice).unwrap_or_default();
            Ok(if view == "export" { describe_export(lines) } else { describe_series(raw, lines, now_ms) })
        }
        other => anyhow::bail!("App record: view is \"summary\", \"series\" or \"export\", not {other:?}"),
    }
}

async fn get(cx: &ToolCtx<'_>, query: &[(&str, &str)]) -> Result<Value> {
    let url = format!("{}/api/v1/app/record", cx.local_url.trim_end_matches('/'));
    let resp = cx.client.get(&url).header(AUTH_KEY_HEADER, cx.auth_key).query(query).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App record: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(body)
}

/// A `[min, median, p95, max]` as numbers.
fn stat(v: &Value) -> Option<[u64; 4]> {
    let a = v.as_array()?;
    Some([a.first()?.as_u64()?, a.get(1)?.as_u64()?, a.get(2)?.as_u64()?, a.get(3)?.as_u64()?])
}

/// The stat of `key` among a rollup's `parts` or `rows` (a list of `[key, stat]`).
fn keyed(r: &Value, list: &str, key: &str) -> Option<[u64; 4]> {
    r[list].as_array()?.iter().find(|p| p[0].as_str() == Some(key)).and_then(|p| stat(&p[1]))
}

fn parts_of(r: &Value) -> String {
    let part = |k: &str| keyed(r, "parts", k).map_or(0, |s| s[1]);
    format!(
        "core {}, panes {}, agents {}, not placed {}",
        mb(part("core")),
        mb(part("panes")),
        mb(part("agents")),
        mb(part("not_placed"))
    )
}

fn span(ms: u64) -> String {
    ago(0, &json!(-(ms as i64)))
}

/// What the record holds, one line.
fn stored(inv: &Value) -> String {
    let (days, months) = (inv["raw_days"].as_u64().unwrap_or(0), inv["rollup_months"].as_u64().unwrap_or(0));
    if days == 0 && months == 0 {
        return "The record on this machine is empty: it starts with the first line five minutes after AgentMux starts.".to_string();
    }
    format!(
        "The record on this machine (it never leaves the computer): five-minute lines for {days} day(s) since {} ({}), hourly rollups for {months} month(s) since {} ({}).",
        inv["raw_first_day"].as_str().unwrap_or("-"),
        mb(inv["raw_bytes"].as_u64().unwrap_or(0)),
        inv["rollup_first_month"].as_str().unwrap_or("-"),
        mb(inv["rollup_bytes"].as_u64().unwrap_or(0))
    )
}

/// The newest line, as a sentence.
fn latest(line: &Value, now_ms: i64) -> String {
    let am = &line["am"];
    let part = |k: &str| am["parts"].as_array().and_then(|p| p.iter().find(|x| x["k"].as_str() == Some(k))).and_then(|x| x["mem"].as_u64()).unwrap_or(0);
    let mut out = format!(
        "Newest line, {} ago (srv {}, up {}): AgentMux {} in {} processes (core {}, panes {}, agents {}, not placed {}).",
        ago(now_ms, &line["t"]),
        s(line, "v"),
        span(line["up_s"].as_u64().unwrap_or(0) * 1000),
        mb(am["total"].as_u64().unwrap_or(0)),
        am["procs"],
        mb(part("core")),
        mb(part("panes")),
        mb(part("agents")),
        mb(part("not_placed"))
    );
    let m = &line["machine"];
    if let Some(mem) = m["mem"].as_u64() {
        out.push_str(&format!(" The machine: {} private", gb(mem)));
        if let Some(total) = m["mem_total"].as_u64() {
            out.push_str(&format!(" of {}", gb(total)));
        }
        if let Some(free) = m["disk"][0].as_u64() {
            out.push_str(&format!(", record drive {} free", gb(free)));
        }
        out.push('.');
    }
    out
}

/// The rollup nearest `target_ms`.
fn nearest(rolls: &[Value], target_ms: u64) -> Option<&Value> {
    rolls.iter().min_by_key(|r| r["h"].as_u64().unwrap_or(0).abs_diff(target_ms))
}

/// AgentMux's size by the hour and what grew, from the hourly rollups.
fn trend(rolls: &[Value], now_ms: i64) -> String {
    let Some(last) = rolls.last() else {
        return "No hour has been rolled up yet, so there is no trend to read (the first rollup is written when the first full hour ends).".to_string();
    };
    let last_h = last["h"].as_u64().unwrap_or(0);
    let mut out = String::from("AgentMux's total by the hour (median, with min to max):");
    for back in [0u64, 1, 6, 24, 168] {
        let target = last_h.saturating_sub(back * 3_600_000);
        let Some(r) = nearest(rolls, target).filter(|r| r["h"].as_u64().unwrap_or(0).abs_diff(target) < 3_600_000) else { continue };
        if let Some([min, med, _, max]) = stat(&r["am"]) {
            let when = if back == 0 { format!("the last hour ({} ago)", ago(now_ms, &r["h"])) } else { format!("{} before it", span(back * 3_600_000)) };
            out.push_str(&format!("\n- {when}: {} ({} to {})", mb(med), mb(min), mb(max)));
        }
    }
    let Some(then) = nearest(rolls, last_h.saturating_sub(DAY_MS as u64)).filter(|r| r["h"].as_u64().unwrap_or(0) + 3_600_000 <= last_h) else {
        return out;
    };
    let then_h = then["h"].as_u64().unwrap_or(0);
    let restarts: u64 = rolls.iter().filter(|r| r["h"].as_u64().unwrap_or(0) > then_h).map(|r| r["restarts"].as_u64().unwrap_or(0)).sum();
    if let (Some(a), Some(b)) = (stat(&then["am"]), stat(&last["am"])) {
        let delta = b[1] as i64 - a[1] as i64;
        out.push_str(&format!(
            "\nOver {}: {}{} ({:+.1}%).{}",
            span(last_h - then_h),
            if delta >= 0 { "+" } else { "-" },
            mb(delta.unsigned_abs()),
            if a[1] == 0 { 0.0 } else { delta as f64 * 100.0 / a[1] as f64 },
            if restarts > 0 { format!(" srv restarted {restarts} time(s) in that time, so a drop is a restart, not a fix.") } else { String::new() }
        ));
    }
    out.push_str(&grown(then, last));
    out
}

/// The rows (and parts) whose median grew most between two hours.
fn grown(then: &Value, now: &Value) -> String {
    let mut moves: Vec<(String, i64)> = Vec::new();
    for list in ["parts", "rows"] {
        for p in now[list].as_array().into_iter().flatten() {
            let (Some(key), Some(b)) = (p[0].as_str(), stat(&p[1])) else { continue };
            let a = keyed(then, list, key).map_or(0, |s| s[1]);
            moves.push((key.to_string(), b[1] as i64 - a as i64));
        }
    }
    moves.retain(|(_, d)| *d > 0);
    moves.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if moves.is_empty() {
        return "\nNo part or row of the largest six grew.".to_string();
    }
    let list: Vec<String> = moves.iter().take(4).map(|(k, d)| format!("{k} +{}", mb(*d as u64))).collect();
    format!("\nGrew most (median, same two hours): {}.", list.join(", "))
}

/// `summary`: what the record holds, the newest line, and the trend.
pub(super) fn describe_summary(inventory: &Value, newest: Option<&Value>, rolls: &[Value], now_ms: i64) -> String {
    let mut out = stored(inventory);
    if let Some(line) = newest {
        out.push('\n');
        out.push_str(&latest(line, now_ms));
    }
    out.push('\n');
    out.push_str(&trend(rolls, now_ms));
    out.push_str("\nView \"series\" lists the hours or lines, \"export\" returns them as stored, to attach to a bug.");
    out
}

/// `series`: one line per hour (or per five minutes with kind raw), oldest first.
pub(super) fn describe_series(raw: bool, lines: &[Value], now_ms: i64) -> String {
    if lines.is_empty() {
        return "The record has nothing in that window yet.".to_string();
    }
    let mut out = String::from(if raw { "Five-minute lines, oldest first:" } else { "Hourly rollups (median, with min to max), oldest first:" });
    for l in lines {
        if raw {
            out.push_str(&format!(
                "\n- {} ago: AgentMux {} in {} processes; machine {} private; srv up {}",
                ago(now_ms, &l["t"]),
                mb(l["am"]["total"].as_u64().unwrap_or(0)),
                l["am"]["procs"],
                mb(l["machine"]["mem"].as_u64().unwrap_or(0)),
                span(l["up_s"].as_u64().unwrap_or(0) * 1000)
            ));
        } else if let Some([min, med, _, max]) = stat(&l["am"]) {
            let restarts = l["restarts"].as_u64().unwrap_or(0);
            out.push_str(&format!(
                "\n- {} ago: AgentMux {} ({} to {}); {}; srv up {}, {} line(s){}",
                ago(now_ms, &l["h"]),
                mb(med),
                mb(min),
                mb(max),
                parts_of(l),
                span(l["up_s"].as_u64().unwrap_or(0) * 1000),
                l["n"],
                if restarts > 0 { format!(", restarted {restarts}x") } else { String::new() }
            ));
        }
    }
    out
}

/// `export`: the lines as stored, one JSON object per line, cut to [`EXPORT_BYTES`] (the newest are kept).
pub(super) fn describe_export(lines: &[Value]) -> String {
    if lines.is_empty() {
        return "The record has nothing in that window yet.".to_string();
    }
    let texts: Vec<String> = lines.iter().map(Value::to_string).collect();
    let mut kept = 0usize;
    let mut bytes = 0usize;
    for t in texts.iter().rev() {
        if bytes + t.len() + 1 > EXPORT_BYTES {
            break;
        }
        bytes += t.len() + 1;
        kept += 1;
    }
    let cut = texts.len() - kept;
    let note = if cut > 0 { format!("{cut} older line(s) left out to fit {} KB; ask for a shorter window (hours or day) to get them. ", EXPORT_BYTES / 1000) } else { String::new() };
    format!("{note}{kept} line(s), one JSON object each (numbers and stable keys only):\n{}", texts[cut..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3_600_000;

    fn roll(h: u64, total: u64, core: u64, restarts: u64) -> Value {
        json!({
            "h": h, "n": 12, "v": "0.60.31", "up_s": 7_200, "restarts": restarts,
            "am": [total - 10, total, total + 5, total + 10],
            "parts": [["core", [core, core, core, core]], ["panes", [1, 1, 1, 1]], ["agents", [2, 2, 2, 2]], ["not_placed", [0, 0, 0, 0]]],
            "rows": [["agent:b1", [core, core, core, core]]]
        })
    }

    #[test]
    fn an_empty_record_says_so_and_a_young_one_says_there_is_no_trend() {
        let text = describe_summary(&json!({ "raw_days": 0, "rollup_months": 0 }), None, &[], 1_000);
        assert!(text.contains("is empty") && text.contains("No hour has been rolled up yet"), "{text}");
    }

    #[test]
    fn growth_over_a_day_is_named_with_the_rows_that_grew_and_a_restart_is_said() {
        let now = 1_000 * H;
        let mut rolls: Vec<Value> = (0..30).map(|i| roll(now - (31 - i) * H, 1_000_000_000 + i * 10_000_000, 400_000_000 + i * 8_000_000, 0)).collect();
        rolls[10]["restarts"] = json!(1);
        let inv = json!({ "raw_days": 3, "raw_bytes": 3_000_000, "raw_first_day": "2026-11-14", "rollup_months": 1, "rollup_bytes": 50_000, "rollup_first_month": "2026-11" });
        let newest = json!({ "t": now - 120_000, "v": "0.60.31", "up_s": 90_000, "am": { "total": 1_300_000_000u64, "procs": 41, "parts": [{ "k": "core", "mem": 600_000_000u64, "n": 3 }] }, "machine": { "mem": 12_000_000_000u64, "mem_total": 32_000_000_000u64, "disk": [200_000_000_000u64, 500_000_000_000u64] } });
        let text = describe_summary(&inv, Some(&newest), &rolls, now as i64);
        assert!(text.contains("five-minute lines for 3 day(s) since 2026-11-14") && text.contains("hourly rollups for 1 month(s)"), "{text}");
        assert!(text.contains("Newest line, 2 min ago (srv 0.60.31, up 25 h 0 min): AgentMux 1239.8 MB in 41 processes"), "{text}");
        assert!(text.contains("Over 24 h 0 min: +228.9 MB (+22.9%)"), "{text}");
        assert!(text.contains("srv restarted 1 time(s)"), "{text}");
        assert!(text.contains("Grew most") && text.contains("core +183.1 MB") && text.contains("agent:b1 +183.1 MB"), "{text}");
    }

    #[test]
    fn a_series_lists_each_hour_with_its_parts_and_an_export_keeps_the_newest_lines_that_fit() {
        let now = 100 * H;
        let rolls = vec![roll(now - 2 * H, 1_000_000_000, 400_000_000, 0), roll(now - H, 1_010_000_000, 410_000_000, 2)];
        let text = describe_series(false, &rolls, now as i64);
        assert!(text.contains("2 h 0 min ago") && text.contains("core 381.5 MB") && text.contains("restarted 2x"), "{text}");
        assert_eq!(describe_series(true, &[], 0), "The record has nothing in that window yet.");

        let big: Vec<Value> = (0..400).map(|i| json!({ "h": i, "pad": "x".repeat(1_000) })).collect();
        let text = describe_export(&big);
        assert!(text.contains("older line(s) left out to fit 200 KB"), "{}", &text[..120]);
        assert!(text.len() < EXPORT_BYTES + 400, "{}", text.len());
        assert!(text.contains("\"h\":399") && !text.contains("\"h\":0,"), "the newest lines are kept");
    }
}
