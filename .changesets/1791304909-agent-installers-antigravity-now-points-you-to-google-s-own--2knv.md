---
type: patch
---

Agent installers: Antigravity now points you to Google's own installer (it was never on npm, so every install failed) and is found right after installing; Claude, Qwen, Gemini and Pi check for the Node.js version they need before installing; Gemini's card says it needs a Gemini API key; Pi no longer shows another product's name and logo; failed installs are now logged with their npm output.
