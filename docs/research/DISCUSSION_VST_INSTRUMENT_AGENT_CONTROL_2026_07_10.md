# Discussion: AI Agent Control of VST Instruments (Serum 2 case study)

**Date:** 2026-07-10
**Status:** Discussion — no proposed AgentMux implementation, findings only
**Context:** Raised while exploring whether an AgentMux agent could drive a music-production workflow (specifically Xfer Serum 2) end-to-end — either via an existing MCP server, or via a "computer use" style GUI-automation loop. Two research passes (external web research, not a codebase investigation) are summarized here.

---

## 1. Question

Could an AgentMux agent (or any AI agent) meaningfully control a synthesizer VST like Serum 2 — tweak knobs, design patches, automate parameters — either through (a) an existing MCP server, or (b) general "computer use" GUI automation? This matters for AgentMux specifically because AgentMux has no built-in computer-use primitive today (see §5) — any answer here informs whether that's worth building, and what it could and couldn't do if built.

---

## 2. The MCP ecosystem for DAWs/VSTs today

**No dedicated MCP server exists for Serum 2.** What exists instead:

- **Ableton Live** has by far the richest MCP ecosystem — at least six independent community projects, ranging from browser/preset-loading only (`ahujasid/ableton-mcp`, ~2,800★, no VST parameter control) to full arbitrary-Python-eval against Ableton's Live Object Model (`bschoepke/ableton-live-mcp`, ~196★ — the one project that explicitly names Serum as usable through it, only because it lets an agent eval Python against Ableton, not because it understands Serum specifically) to OSC-based control (`Simon-Kansara/ableton-live-mcp-server`, stale since March 2025) to MIDI-clip-only editing (`daw-mcp`, explicitly out of scope for plugin parameters).
- **Bitwig Studio** has one OSC-based bridge (`bitwig-mcp-server`), stalled since April 2025.
- **Reaper** has no dedicated MCP server at all, despite a long history of scriptability (ReaScript/OSC) that seems well-suited to it — a real gap, not confirmed as intentional.
- **Generic VST/VST3 hosting** (DAW-agnostic): `agrathwohl/carla-mcp-server` wraps Carla (a plugin host supporting VST2/VST3/LV2/LADSPA/DSSI/AU), exposing 45 tools across plugin loading and generic parameter automation. This is the closest thing to a "control any VST via MCP" solution today, but it's small (~13★) and treats every plugin's parameters as anonymous numeric slots — no semantic understanding of what Serum's "Filter Cutoff" knob is versus its neighbor.
- **Non-agentic, offline preset generators** exist for Serum specifically (PresetLab.ai, Tdub206/Serum-Preset-Generator) — these turn a text prompt or JSON config into a `.serumPreset` file, but have no live connection, no MCP interface, and no way for an agent to iterate against real-time audio feedback.

**Bottom line:** if you load Serum inside Ableton today, an MCP-driven agent can technically twiddle its parameters, but only as blind numeric indices via Ableton's generic device-parameter API — not by name, and not with any concept of what a given knob does.

---

## 3. Why Serum 2 is unusually hard to give an agent semantic control over

Xfer deliberately did not give Serum a static, complete VST3 parameter list. Per a Xfer developer (XferRecords forum thread on Live 12.1 automation), a full modulation-matrix parameter table would balloon toward roughly a million theoretical slots (any source → any destination), which would choke DAWs trying to enumerate it up front. Instead, Serum registers a parameter **dynamically** — it only becomes visible to the host's automation/API surface *after* a human right-clicks → Automate (or uses MIDI-Learn → Enable Host Automation) on that specific control, per plugin instance. There's also a known, currently-unresolved Ableton Live 12.1 bug where FX-page parameters beyond filter-1 cutoff frequently don't register at all.

Core knobs (oscillator pitch/level/pan, wavetable position, filter cutoff, macros, LFO rate, envelope stages, reverb) do have stable, human-readable names once activated — third-party MIDI controller products (MP MIDI Controller, Serumify 2) already auto-map Serum 1/2 this way. But there is no way to get the *complete* set up front; an agent would need to interactively "warm up" each parameter (simulate the Configure/Automate click) before it becomes queryable or settable, and that's before accounting for the host-side automation bugs above.

---

## 4. Is there a documented headless/programmatic path around the GUI requirement?

**Researched directly and came up empty.** Two of the most plausible candidate mechanisms were checked and both failed independent verification:

- **"VST3 requires a fixed parameter list at init"** (which would mean Serum's dynamic behavior is non-compliant, implying a documented alternate path) — refuted (1-2 vote).
- **`IEditController::setParamNormalized()`** — a real, documented VST3 SDK call that lets a host set an *already-registered* parameter's value without a GUI click — was proposed as at least a partial workaround. This was explicitly rejected (0-3 vote) as a reliable documented path even for that narrower claim. No source could confirm it reliably applies to Serum's dynamically-registered parameters before they've been activated once via the GUI.

No sysex/MIDI trick, undocumented API, or community reverse-engineering project surfaced either. This should be read as **"no evidence found," not "proven impossible"** — Serum's `.serumPreset`/`.fxp` file format is proprietary and undocumented (CBOR+Zstandard-compressed per community reverse-engineering, per the Tdub206 preset generator's approach of writing that format directly rather than reading it back), so a side-channel via direct preset-file manipulation (write the file, reload the plugin) remains a plausible unexplored option, distinct from live parameter automation.

Open question worth revisiting: could a human do a one-time GUI-driven "activation pass" over Serum's full control set, after which an agent could drive everything from `setParamNormalized()`-equivalent calls without further GUI interaction? Not confirmed either way by this research.

---

## 5. OS-level accessibility automation (pywinauto, Windows UI Automation)

Independent of VST3-specific questions: Serum's GUI is fully custom-rendered (owner-drawn, not native Win32/WPF widgets). Windows UI Automation only auto-exposes standard native controls — a custom-drawn control is "largely opaque" to UIA clients (bounding box at best, no parameter names/values) unless the plugin vendor explicitly implements UI Automation provider interfaces. Xfer has not done this (nor have most audio plugin vendors — JUCE, the C++ framework underlying many synths, only added UIA support in JUCE 6, and it must be explicitly enabled). `pywinauto` inherits this limitation exactly — it has two backends (raw Win32, UIA), no vision-based fallback, and its own documentation has no guidance for custom-drawn controls; unsupported controls fall back to blind coordinate-based clicking, the same failure mode as vision-based agents (see §6).

---

## 6. How good are current "computer use" tools at this kind of task anyway?

Even setting aside Serum's dynamic-parameter quirk, the broader "can an AI agent precisely drag a knob to an exact value" question has a fairly clear, discouraging answer as of mid-2026, from three independent benchmark papers:

- **FineState-Bench** and its follow-up: fine-grained/precise-state manipulation is the *weakest* capability category for current GUI agents — the single best fine-grained single-action score across every tested model/platform combination is **32.8%**.
- **DragOn** (a benchmark purpose-built for drag interactions — text highlighting, cell selection, resizing, slider manipulation): no frontier VLM-based agent clears **30%** success on drag-based grounding overall; even the best slider-manipulation sub-scores (the closest proxy to a knob) top out at 57.2% (Claude Opus 4.7) and 45.6% (GPT-5.4) — respectable-sounding numbers, but nowhere near reliable enough for "land on an exact numeric value."
- **OSExpert-Eval** (fine-grained manipulation in real desktop apps — GIMP, LibreOffice): leading agents, including OpenAI's own Computer-Use-Preview model, score **0.00–0.10** — essentially zero.

Microsoft's UFO/UFO2 mitigates some of this by mixing native accessibility-API calls with GUI clicks rather than relying purely on pixel-level dragging — but that hybrid advantage only applies to controls exposed via UIA/Win32/WinCOM in the first place, which (per §5) a custom-rendered synth GUI is not. Open-source vision-agent projects like OpenAdapt report good numbers only on coarse click-navigation tasks (a shared-entry-point macOS Settings benchmark), with no published slider/knob/drag-precision results at all.

**Takeaway:** as of mid-2026, no available tool or framework — Anthropic's Computer Use, OpenAI's CUA/Operator, UFO, OpenAdapt, or OS accessibility automation — reliably achieves pixel-precise, exact-value control of a custom-rendered GUI control like a Serum knob. Clicking a button or typing text is a solved problem for these tools; continuous-value dragging is not.

---

## 7. Where this leaves AgentMux specifically

AgentMux's own tool surface (`Shell`, `FocusWindow`, `OpenEditor`, `Layout`, `NewTab`, etc.) is entirely about managing AgentMux's own UI — there is no mouse/keyboard/screenshot "computer use" primitive today that could click, drag, or read pixels in a third-party window like Serum's. Building one would face the ceiling described in §6 for *any* fine-motor knob-dragging task, not just Serum — so a computer-use feature would be much more reliably useful for click/type-heavy workflows (form filling, navigating settings dialogs, launching other apps) than for audio-plugin sound design.

For Serum/VST control specifically, the more tractable near-term path if this becomes a real use case is **not** computer-use at all, but the same one every existing community MCP project already takes: host the plugin inside a DAW (or a generic host like Carla) and drive it through the DAW/host's own parameter API — accepting that this only offers anonymous numeric control (§2) unless someone builds a semantic name/mapping layer on top, which does not exist anywhere today.

---

## 8. Open questions for a future pass

- Can Serum's `.serumPreset`/`.fxp` binary format be written directly (bypassing live automation entirely) as a reliable headless authoring path, distinct from real-time parameter tweaking?
- Does a one-time human-driven "activate every parameter" pass on a given Serum instance make the rest of the session's control fully programmatic (via `setParamNormalized()`-equivalent calls), or does the dynamic-registration behavior reset per session/per plugin-reload?
- Would an iterative closed-loop approach (screenshot → diff → re-drag, rather than one blind action) meaningfully close the drag-precision gap for a computer-use tool, and do any of the surveyed benchmarks even measure multi-attempt accuracy?
- Has anyone built a UI Automation provider *shim* at the host level (mapping a plugin's rendered pixels to a synthetic accessibility tree) that would sidestep the need for the plugin vendor itself to add UIA support?

---

## 9. Sources

**MCP/VST ecosystem:**
- https://github.com/ahujasid/ableton-mcp
- https://github.com/jpoindexter/ableton-mcp
- https://github.com/uisato/ableton-mcp-extended
- https://github.com/bschoepke/ableton-live-mcp
- https://github.com/Simon-Kansara/ableton-live-mcp-server
- https://github.com/ptaczek/daw-mcp
- https://github.com/WeModulate/bitwig-mcp-server
- https://github.com/agrathwohl/carla-mcp-server
- https://presetlab.ai/
- https://github.com/Tdub206/Serum-Preset-Generator

**Serum 2 dynamic parameters / VST3 internals:**
- https://www.xferrecords.com/forums/general/serum-2-automation-on-live-12-1
- https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Parameters+Automation/Index.html
- https://help.ableton.com/hc/en-us/articles/209067009-Automating-plug-in-parameters-that-can-t-be-configured
- https://mpmidi.com/serum-2-controller
- https://remotify.io/product/serumify-2-midi-fighter-twister/

**OS accessibility automation:**
- https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-providersoverview
- https://github.com/pywinauto/pywinauto
- https://github.com/pywinauto/pywinauto/wiki/How-to-enable-accessibility-(tips-and-tricks)
- https://github.com/yinkaisheng/Python-UIAutomation-for-Windows
- https://forum.juce.com/t/windows-ui-automation/57719

**Computer-use precision benchmarks:**
- https://arxiv.org/html/2508.09241 (FineState-Bench)
- https://arxiv.org/html/2606.06322 (DragOn)
- https://arxiv.org/pdf/2603.07978 (OSExpert-Eval)
- https://github.com/microsoft/UFO
- https://github.com/OpenAdaptAI/OpenAdapt
