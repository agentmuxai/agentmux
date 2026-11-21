// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Runs before first paint (index.html loads it as a classic, synchronous script from the
// page's own origin). It reads only query parameters the desktop host appends to a window's URL.

// Set --window-opacity before first paint so the translucent SCSS default
// (0.45) doesn't flash for window:transparent=false users.
// The host appends window_transparent=0|1 to the URL at launch time.
(function () {
    var t = new URLSearchParams(location.search).get("window_transparent");
    if (t === "0") {
        document.documentElement.style.setProperty("--window-opacity", "1");
    }
})();

// SPEC_PILLAR1_STEP4 Phase 4 — show the "Restoring session..." headline
// before first paint when this window was opened by the crash-reproject
// driver (reproject_from_snapshot / reproject_from_srv in
// crates/cef/src/commands/window/creation.rs append &restoring=1 to
// the URL; an interactive "New Window"/"New Subwindow" never does).
(function () {
    if (new URLSearchParams(location.search).get("restoring") === "1") {
        document.addEventListener("DOMContentLoaded", function () {
            var el = document.getElementById("startup-loading-headline");
            if (el) {
                el.style.display = "block";
            }
        });
    }
})();

// A start that takes a while must say so. The splash is a pulsing mark and no words, so a start that waits on a slow
// server (a first start on a fresh home, a busy machine) looks like a blank page, and a report of one has nothing to
// read. After a few seconds, if the splash is still up, it names what it is doing; after a longer wait it says what
// a person can do. It only adds words: nothing here changes how long the start waits or what it waits for. A window
// that is opened to restore a session has its own headline, and a pre-warmed pool window waits hidden, for hours.
(function () {
    if (new URLSearchParams(location.search).get("restoring") === "1") return;
    function say(text) {
        var splash = document.getElementById("startup-loading");
        var headline = document.getElementById("startup-loading-headline");
        if (!splash || !headline) return;
        if (splash.classList.contains("fading") || document.documentElement.hasAttribute("data-pool-window")) return;
        headline.textContent = text;
        headline.style.display = "block";
    }
    setTimeout(function () {
        say("Connecting to AgentMux\u2026");
    }, 3000);
    setTimeout(function () {
        say("Still connecting. If this stays, reload the page.");
    }, 15000);
})();
