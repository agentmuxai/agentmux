// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The plan's list of every place an agent's old slug is stored (K1–K13 of
//! the spec's §3), read through each step's own before-image, so the plan
//! shows exactly what a run would touch and nothing it wouldn't.

use serde_json::Value;

use super::home::{Home, Install};
use super::journal::Step;
use super::plan::Def;
use super::steps::{before, Agent, Ctx};
use crate::backend::rpc_types::ForkSlugLocation;

fn loc(key: &str, place: impl Into<String>, value: impl Into<String>) -> ForkSlugLocation {
    ForkSlugLocation { key: key.to_string(), place: place.into(), value: value.into() }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub(super) fn locations(home: &Home, installs: &[Install], def: &Def, new_slug: &str) -> Result<Vec<ForkSlugLocation>, String> {
    let ctx = Ctx { home, installs, run_dir: Default::default(), backups: &[], retired_at: 0 };
    let a = Agent { uid: def.uid.clone(), name: def.name.clone(), old: def.slug.clone(), new: new_slug.to_string() };
    let mut out = vec![loc(
        "K2",
        def.path.display().to_string(),
        if def.former.is_empty() { def.slug.clone() } else { format!("{} (former: {})", def.slug, def.former.join(", ")) },
    )];
    for row in before(&ctx, &a, Step::S2)?.as_array().into_iter().flatten() {
        out.push(loc("K1", text(&row["db"]), row["slug"].as_str().unwrap_or("(no slug)")));
    }
    for db in before(&ctx, &a, Step::S3)?.as_array().into_iter().flatten() {
        for r in db["rows"].as_array().into_iter().flatten() {
            let key = if text(&r[0]) == "db_agent_wan_keys" { "K5" } else if text(&r[0]) == "db_agent_lan_keys" { "K4" } else { "K3" };
            out.push(loc(key, text(&db["db"]), format!("{} row under {}", text(&r[0]), text(&r[1]))));
        }
    }
    for db in before(&ctx, &a, Step::S4)?.as_array().into_iter().flatten() {
        for k in db["keys"].as_array().into_iter().flatten() {
            out.push(loc("K5", text(&db["db"]), format!("WAN key under {}", text(k))));
        }
        let published = db["publications"].as_array().map_or(0, Vec::len);
        if published > 0 {
            out.push(loc("K5", text(&db["db"]), format!("published to {published} account directory(ies)")));
        }
    }
    for c in before(&ctx, &a, Step::S5)?.as_array().into_iter().flatten() {
        out.push(loc("K7", text(c), "MuxBus credential, kept for the grace subscription"));
    }
    let name_only = before(&ctx, &a, Step::S6)?;
    for u in name_only["updates"].as_array().into_iter().flatten() {
        out.push(loc("K13", text(&u["db"]), format!("{} {} names {}", text(&u["table"]), text(&u["id"]), text(&u["set"][0][1]))));
    }
    for g in name_only["grants"].as_array().into_iter().flatten() {
        out.push(loc("K13", text(&g["db"]), format!("trust grant {} -> {} ({})", text(&g["agent_id"]), text(&g["peer"]), text(&g["tier"]))));
    }
    let identity = before(&ctx, &a, Step::S7)?;
    for r in identity["rows"].as_array().into_iter().flatten() {
        let what = if text(&r["table"]) == "db_client" { "layout snapshot" } else { "pane" };
        let keys: Vec<String> = r["changes"].as_array().into_iter().flatten().map(|c| text(&c["key"])).collect();
        out.push(loc("K12", text(&r["db"]), format!("{what} {}: {}", text(&r["oid"]), keys.join(", "))));
    }
    for f in identity["files"].as_array().into_iter().flatten() {
        out.push(loc("K12", text(f), format!("signs as {}", a.old)));
    }
    let files = before(&ctx, &a, Step::S8)?;
    for m in files["moves"].as_array().into_iter().flatten() {
        let key = if text(&m["from"]).ends_with(".cwd") { "K11" } else { "K10" };
        let note = if m["to_existed"].as_bool() == Some(true) { " (the new one exists too: both kept)" } else { "" };
        out.push(loc(key, text(&m["from"]), format!("moves to {}{note}", text(&m["to"]))));
    }
    for d in files["deletes"].as_array().into_iter().flatten() {
        out.push(loc("K9", text(d), "stale registration, deleted"));
    }
    Ok(out)
}
