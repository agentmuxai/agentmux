// Reports what this page receives to the demo server's /e, for a scripted pass to check.
const send = (k, v) => navigator.sendBeacon("/e?k=" + k + "&v=" + encodeURIComponent(v));
addEventListener("load", () => send("load", location.pathname + " " + innerWidth + "x" + innerHeight));
addEventListener("mousedown", (e) => send("click", Math.round(e.clientX) + "," + Math.round(e.clientY)));
addEventListener("keydown", (e) => send("key", e.key));
addEventListener("input", (e) => send("input", e.target.value));
addEventListener("scroll", () => send("scroll", Math.round(scrollY)));
addEventListener("resize", () => send("resize", innerWidth + "x" + innerHeight));
