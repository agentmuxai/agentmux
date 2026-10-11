# Acknowledgements

AgentMux builds on the work of many open-source projects and the people who
maintain them. Thank you.

## Origin

AgentMux is a fork of **Wave Terminal**, originally developed by
**Command Line Inc.**, licensed under the Apache License 2.0.

- Upstream: https://github.com/wavetermdev/waveterm
- See also: [NOTICE](./NOTICE)

## Key Third-Party Software

The shipped binaries include or link against, among others:

**Runtime & UI**

- Chromium Embedded Framework (CEF) — BSD-3-Clause
- xterm.js — MIT
- CodeMirror 6 — MIT
- SolidJS — MIT
- [Font Awesome Free](https://fontawesome.com) 6.7.2 — icons CC BY 4.0, fonts
  SIL OFL 1.1, CSS MIT. Copyright Fonticons, Inc. Shipped as web fonts in
  `public/fontawesome/` (licence: `public/fontawesome/LICENSE.txt`). The
  `custom-icons` font there holds Wave Terminal's own glyphs (Apache 2.0).
- Fonts: Hack Nerd Font Mono (Hack: MIT and Bitstream Vera licence; Nerd
  Fonts: OFL 1.1 and MIT), Inter (SIL OFL 1.1), and DejaVu Sans Mono glyphs in
  the launcher's splash screen (Bitstream Vera licence)
- Windows packages also include jq (MIT, with Oniguruma, BSD-2-Clause) and
  ripgrep (MIT or Unlicense)

**Rust backend**

- Tokio, Axum, SQLx, portable-pty, serde, tracing, and many others — see
  `Cargo.toml` and `Cargo.lock`

**Build & test**

- Vite, Vitest, Prettier, ESLint, Stylelint — MIT / various permissive
- [Task](https://taskfile.dev) — MIT

Every AgentMux package carries the full list, with each component's licence
text, in `THIRD_PARTY_NOTICES.txt` in the app's `frontend/licenses/` folder,
next to AgentMux's own `LICENSE.txt` and `NOTICE.txt`. The build generates it
(`scripts/third-party-notices.mjs`) from what it actually ships: the
JavaScript packages in the bundle, the Rust crates in the programs, and the
fonts and tools in `licenses/bundled/`. A production build writes it to
`dist/frontend/licenses/`.

## License Compliance

If you redistribute AgentMux, you must comply with the Apache License 2.0
(see [LICENSE](./LICENSE)), preserve [NOTICE](./NOTICE), and keep the
third-party notices described above with every copy.

## Reporting Attribution Issues

If your work is included here without proper attribution, please email
**legal@agentmux.ai** and we will fix it promptly.
