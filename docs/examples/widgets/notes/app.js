// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentMuxError, connect } from "/agentmux/widget-sdk/v1.js";

const am = await connect();
const list = document.getElementById("notes");
const empty = document.getElementById("empty");

// Each note is one storage key, `note:<time>`, so adding one never rewrites
// the others.
async function load() {
    const keys = await am.storage.list("note:");
    const notes = [];
    for (const key of keys) notes.push({ key, ...(await am.storage.get(key)) });
    return notes.sort((a, b) => b.at - a.at);
}

async function render() {
    const notes = await load();
    list.replaceChildren(
        ...notes.map((note) => {
            const li = document.createElement("li");
            const text = Object.assign(document.createElement("span"), { className: "text", textContent: note.text });
            const copy = Object.assign(document.createElement("button"), { textContent: "Copy" });
            copy.onclick = async () => {
                await am.clipboard.writeText(note.text);
                await am.ui.toast("Copied", "success");
            };
            const del = Object.assign(document.createElement("button"), { textContent: "Delete" });
            del.onclick = async () => {
                await am.storage.delete(note.key);
                await render();
            };
            li.append(text, copy, del);
            return li;
        })
    );
    empty.hidden = notes.length > 0;
    await am.ui.setTitle(`Notes (${notes.length})`);
}

document.getElementById("add").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const box = document.getElementById("text");
    const text = box.value.trim();
    if (!text) return;
    const at = Date.now();
    try {
        await am.storage.set(`note:${at}`, { text, at });
    } catch (e) {
        // Over the 5 MB a widget may keep, for instance.
        if (e instanceof AgentMuxError) return am.ui.toast(e.message, "error");
        throw e;
    }
    box.value = "";
    await render();
});

// Export and import from the header, as JSON files the user picks.
await am.ui.setHeaderActions([
    { id: "export", icon: "file-export", title: "Export the notes" },
    { id: "import", icon: "file-import", title: "Import notes" },
]);
am.on("action", async ({ id }) => {
    try {
        if (id === "export") {
            const notes = (await load()).map(({ text, at }) => ({ text, at }));
            const saved = await am.files.save("notes.json", JSON.stringify(notes, null, 2), "application/json");
            if (saved) await am.ui.toast(`Exported ${notes.length} notes`, "success");
        } else if (id === "import") {
            const [file] = await am.files.pick({ accept: [".json"] });
            const notes = JSON.parse(new TextDecoder().decode(file.bytes()));
            for (const note of notes) await am.storage.set(`note:${note.at}`, { text: String(note.text), at: Number(note.at) });
            await render();
        }
    } catch (e) {
        // A cancelled dialog isn't an error worth showing.
        if (e instanceof AgentMuxError && e.name === "cancelled") return;
        await am.ui.toast(e.message, "error");
    }
});

// Another Notes pane, in any window, added or deleted one.
am.on("storage", () => render());

await render();
