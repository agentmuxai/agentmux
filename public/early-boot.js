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
