// Renders the mock cards. Order is the data order, whatever the column count.
const CONNECTED = [
  { id: "A", n: "AgentMux Cloud", s: ["ok", "Connected · you@example.com"], c: "›" },
  { id: "GH", n: "GitHub", s: ["warn", "2 accounts · 1 needs sign-in again"], c: "⌄", key: "gh" },
];
const AVAILABLE = [
  { id: "G", n: "Google", w: "Gmail, Calendar and Drive for your agents", c: "Connect" },
  { id: "An", n: "Anthropic", w: "Claude, by browser sign-in or an API key", c: "Connect" },
  { id: "Op", n: "OpenAI", w: "ChatGPT sign-in, or an API key", c: "Connect" },
  { id: "AW", n: "AWS", w: "Let agents use an IAM role", c: "Connect" },
  { id: "S", n: "Slack", w: "Read and send Slack messages", c: "Connect" },
  { id: "Ab", n: "Ableton Live", w: "Tool server: control Live from an agent", c: "Add" },
  { id: "TD", n: "TouchDesigner", w: "Tool server: drive a TouchDesigner project", c: "Add" },
  { id: "x", n: "xAI", w: "Grok Build signs in inside the agent", c: "How to connect", plain: true },
  { id: "Mo", n: "Moonshot", w: "Kimi Code signs in inside the agent", c: "How to connect", plain: true },
];
function card(x, open) {
  const state = x.s ? `<div class="state ${x.s[0]}">${x.s[0] === "ok" ? "● " : "▲ "}${x.s[1]}</div>` : `<div class="what">${x.w}</div>`;
  const ctl = x.s ? `<span class="btn quiet">${x.c}</span>` : `<span class="btn ${x.plain ? "" : "accent"}">${x.c}</span>`;
  return `<div class="cell"><div class="card${open ? " open" : ""}${x.s && x.s[0] === "warn" ? " warn" : ""}"><div class="logo">${x.id}</div><div class="text"><div class="name">${x.n}</div>${state}</div><div class="control">${ctl}</div></div></div>`;
}
const DETAIL = (wide) => `<div class="detail" style="--caret:${wide.caret}">
  <div class="${wide.two ? "cols" : ""}"><div>
    <h4>Accounts</h4>
    <div class="inst"><div class="grow"><div>octo-work</div><div class="sub">AgentMux GitHub App · used by 3 agents</div><div class="state ok">● Connected</div></div><span class="btn quiet">Details</span></div>
    <div class="inst"><div class="grow"><div>octo-home</div><div class="sub">Personal access token · used by 1 agent</div><div class="state warn">▲ Needs sign-in again</div></div><span class="btn accent">Reconnect</span></div>
    <div class="actions"><span class="btn accent">Add another account</span><span class="btn danger">Disconnect…</span></div>
    <div class="confirm"><p>Disconnect octo-home? 1 agent uses it and will lose GitHub access.</p><div class="actions"><span class="btn danger">Disconnect</span><span class="btn quiet">Keep</span></div></div>
  </div><div>
    <h4>Where the AgentMux GitHub App is installed</h4>
    <div class="sub" style="padding:4px 0">octo-work: 4 repositories · <a style="color:#7fb2ff">Install on more repositories</a></div>
    <h4>Other ways to connect</h4>
    <div class="inst"><div class="grow"><div>API token</div><div class="sub">A token you make on GitHub, kept in your OS keychain</div></div><span class="btn">Add</span></div>
  </div></div></div>`;
// opts: { cols, max, open: bool, detail: { caret, two } }
function render(opts) {
  const conn = CONNECTED.map((x, i) => card(x, opts.open && x.key === "gh") +
    (opts.open && x.key === "gh" && (i + 1) % opts.cols === 0 ? DETAIL(opts.detail) : "")).join("");
  // The details row goes after the last card of the open card's row (here: after the connected group's row).
  let connHtml = CONNECTED.map((x) => card(x, opts.open && x.key === "gh")).join("");
  if (opts.open) connHtml += DETAIL(opts.detail);
  const avail = AVAILABLE.map((x) => card(x, false)).join("");
  document.getElementById("app").innerHTML = `
  <div class="pane" style="width:${opts.width}px">
    <div class="pane-head"><span class="title">Connectors</span><span class="count">· 2 connected</span>${opts.filter ? '<span class="filter">Filter…</span>' : ""}</div>
    <div class="group">Connected</div>
    <div class="grid" style="--cols:${opts.cols};--max:${opts.max || "1fr"}">${connHtml}</div>
    <div class="group">Available</div>
    <div class="grid" style="--cols:${opts.cols};--max:${opts.max || "1fr"}">${avail}</div>
    <div class="foot">Looking for something else? <a>Add a custom connector</a></div>
  </div>
  <p class="caption">${opts.caption}</p>`;
}
