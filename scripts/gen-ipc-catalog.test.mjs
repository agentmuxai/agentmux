// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for gen-ipc-catalog.mjs: reading the arms of route_command, the stub list, and
// writing the block into the page between its markers.

import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
    BEGIN,
    END,
    generate,
    handlerOf,
    renderCatalog,
    routeCommandEntries,
    splice,
    stubNames,
} from "./gen-ipc-catalog.mjs";
import { routeCommandArms } from "./lib/doc-facts.mjs";

const SAMPLE = `
async fn route_command(state: &Arc<AppState>, cmd: &str, args: &Value) -> Result<Value, String> {
    if is_stub_command(cmd) { return Ok(Value::Null); }
    match cmd {
        // ---- Tier 1: Bootstrap ----
        "get_platform" => Ok(commands::platform::get_platform()),
        "set_zoom" => commands::window::set_zoom(state, args),
        // Close and minimize share a handler.
        "close_window" | "minimize_window" => {
            // a "quoted" word in a comment is not a command
            commands::window::close(state, args)
        }
        "open_it" if cfg!(debug_assertions) => commands::window::open_it().await,
        // ---- Tier 2: Browser ----
        "browser_pane_reload" => {
            state.browser_panes.reload("block", state);
            Ok(serde_json::json!(true))
        },
        "heavy" => {
            tokio::task::spawn_blocking(move || commands::heavy::run()).await.map_err(|e| e.to_string())?
        }
        "gated" => {
            crate::dev_mode::require()?;
            let n = u32::try_from(args["n"].as_u64().unwrap_or(0)).unwrap_or(0);
            let t = std::env::var("X").ok();
            state.panes.inspect(n, t);
            Ok(Value::Null)
        }
        "first" => Ok(a()), // a comment on the line of the arm above is not the next arm's note
        // Two blank-separated comments: only the one against the arm counts.

        // The note of "second".
        "second" => Ok(b()),
        // The note of "third", a long one that has to be cut at a word and not in the middle of one: Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor.
        "third" => Ok(c()),
        // ---- Tier 3: After a banner ----
        // Directly under a banner is this arm's.
        "fourth" => Ok(d()),
        "debug:hang_ui" => Err("a \\"quoted\\" => string".to_string()),
        _ => Err(format!("Unknown command: {}", cmd)),
    }
}
`;

const STUBS = `
pub fn is_stub_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "download_file"
            | "quicklook"
            // Update
            | "install_update"
    )
}
`;

describe("routeCommandEntries", () => {
    const entries = routeCommandEntries(SAMPLE);
    const by = (name) => entries.find((e) => e.names.includes(name));

    it("finds every string arm, an or-pattern as one entry with both names, and not the wildcard", () => {
        expect(entries.flatMap((e) => e.names)).toEqual([
            "get_platform",
            "set_zoom",
            "close_window",
            "minimize_window",
            "open_it",
            "browser_pane_reload",
            "heavy",
            "gated",
            "first",
            "second",
            "third",
            "fourth",
            "debug:hang_ui",
        ]);
        expect(by("close_window")).toBe(by("minimize_window"));
    });

    it("names the handler the arm calls, a method on state, or says the arm holds the code", () => {
        expect(by("get_platform").handler).toBe("commands::platform::get_platform");
        expect(by("browser_pane_reload").handler).toBe("state.browser_panes.reload");
        expect(by("debug:hang_ui").handler).toBe("in the arm");
    });

    it("skips a spawn_blocking wrapper, std and tokio paths, primitives and a guard before the handler", () => {
        expect(by("heavy").handler).toBe("commands::heavy::run");
        expect(by("gated").handler).toBe("state.panes.inspect");
        expect(handlerOf('Ok(Duration::from_millis(5)).and_then(|x| crate::mod_a::work(x))')).toBe("mod_a::work");
        expect(handlerOf("state.auth_key.lock().clone()")).toBe("in the arm");
    });

    it("takes a function handed to spawn_blocking by name, as in the real stop_tab_drag_tracking and store_update_install arms", () => {
        expect(handlerOf("tokio::task::spawn_blocking(crate::commands::store_update::install_update).await")).toBe("commands::store_update::install_update");
        expect(handlerOf("tokio::task::spawn_blocking(commands::drag::stop).await.map_err(|e| e.to_string())?")).toBe("commands::drag::stop");
    });

    it("says whether the arm awaits, hops to a blocking thread, or neither, and whether it takes the arguments", () => {
        expect(by("get_platform").runs).toBe("inline");
        expect(by("open_it").runs).toBe("async");
        expect(by("heavy").runs).toBe("blocking");
        expect(by("set_zoom").args).toBe(true);
        expect(by("get_platform").args).toBe(false);
    });

    it("groups by the banner comment above, and takes the comment above an arm as its note", () => {
        expect(by("get_platform").group).toBe("Tier 1: Bootstrap");
        expect(by("heavy").group).toBe("Tier 2: Browser");
        expect(by("close_window").note).toBe("Close and minimize share a handler.");
        expect(by("close_window").note).not.toContain("quoted");
    });

    it("takes only the comment directly above an arm: not one past a blank line, nor one at the end of the arm above", () => {
        expect(by("first").note).toBe("");
        expect(by("second").note).toBe("The note of \"second\".");
        expect(by("fourth").note).toBe("Directly under a banner is this arm's.");
    });

    it("gives nothing when there is no route_command", () => {
        expect(routeCommandEntries("fn other() {}")).toEqual([]);
    });
});

describe("stubNames", () => {
    it("lists the names of is_stub_command, comments aside", () => {
        expect(stubNames(STUBS)).toEqual(["download_file", "quicklook", "install_update"]);
    });
});

describe("renderCatalog and splice", () => {
    const entries = routeCommandEntries(SAMPLE);
    const block = renderCatalog(entries, ["download_file", "quicklook", "install_update"]);

    it("writes a table per group, counts the commands and lists the stubs", () => {
        expect(block).toContain("#### Tier 1: Bootstrap");
        expect(block).toContain("| `close_window`, `minimize_window` | `commands::window::close` |");
        expect(block).toContain("13 commands the host handles, and 3 names");
        expect(block).toContain("`download_file`, `quicklook`, `install_update`");
    });

    it("leaves a command that is also a stub out of the tables", () => {
        const withStub = renderCatalog(entries, ["heavy"]);
        expect(withStub).not.toContain("| `heavy`");
        expect(withStub).toContain("12 commands the host handles");
    });

    it("cuts a long note at a word, with an ellipsis", () => {
        const row = renderCatalog(entries, []).split("\n").find((l) => l.startsWith("| `third`"));
        const note = row.split("|")[5].trim();
        expect(note.endsWith("…")).toBe(true);
        expect(note.length).toBeLessThanOrEqual(301);
        expect(note).not.toMatch(/\s…$/);
        expect(note.slice(0, -1)).toMatch(/\b\w+$/);
    });

    it("keeps a pipe in a note from splitting the row", () => {
        const e = [{ names: ["a"], group: "G", note: "x | y", handler: "m::f", runs: "inline", args: false }];
        expect(renderCatalog(e, [])).toContain("| x \\| y |");
    });

    it("replaces what is between the markers and keeps the prose around them", () => {
        const page = `before\n\n${BEGIN}\n\nold\n\n${END}\n\nafter\n`;
        const out = splice(page, `${BEGIN}\n\nnew\n\n${END}`);
        expect(out.page).toBe(`before\n\n${BEGIN}\n\nnew\n\n${END}\n\nafter\n`);
        expect(splice(out.page, `${BEGIN}\n\nnew\n\n${END}`).page).toBe(out.page);
    });

    it("refuses a page without the markers", () => {
        expect(splice("no markers", block).error).toMatch(/markers/);
    });
});

describe("the real route_command", () => {
    const ipc = new URL("../crates/cef/src/ipc.rs", import.meta.url);

    it.skipIf(!existsSync(ipc))("reads exactly the arms the docs fact check reads (plus the colon-named debug hook)", () => {
        const src = readFileSync(ipc, "utf8");
        const mine = new Set(routeCommandEntries(src).flatMap((e) => e.names).filter((n) => /^[a-z0-9_]+$/.test(n)));
        expect([...mine].sort()).toEqual([...routeCommandArms(src)].sort());
    });

    it.skipIf(!existsSync(ipc))("names a handler for the two arms that pass it to spawn_blocking without calling it", () => {
        const by = (name) => routeCommandEntries(readFileSync(ipc, "utf8")).find((e) => e.names.includes(name));
        expect(by("stop_tab_drag_tracking").handler).toBe("commands::drag::stop_tab_drag_tracking");
        expect(by("store_update_install").handler).toBe("commands::store_update::install_update");
    });

    it.skipIf(!existsSync(ipc))("generates a block from the repo's own code", () => {
        const g = generate(new URL("..", import.meta.url).pathname);
        expect(g.error).toBeUndefined();
        expect(g.block.startsWith(BEGIN)).toBe(true);
        expect(g.block.endsWith(END)).toBe(true);
    });
});
