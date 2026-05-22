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
- Monaco Editor — MIT
- SolidJS — MIT

**Rust backend**

- Tokio, Axum, SQLx, portable-pty, serde, tracing, and many others — see
  `Cargo.toml` and `Cargo.lock`

**Build & test**

- Vite, Vitest, Prettier, ESLint, Stylelint — MIT / various permissive
- [Task](https://taskfile.dev) — MIT

The complete dependency list with versions and licenses is in:

- `package.json` + `package-lock.json` (JS/TS)
- `Cargo.toml` + `Cargo.lock` (Rust)

To regenerate a full license report:

```bash
# Rust
cargo install cargo-about
cargo about generate about.hbs > licenses-rust.html

# JS
npx license-checker --summary
```

## License Compliance

If you redistribute AgentMux, you must comply with the Apache License 2.0
(see [LICENSE](./LICENSE)) and preserve [NOTICE](./NOTICE). Third-party
licenses for bundled binaries are included in the installer/zip in
`licenses/` (when present).

## Reporting Attribution Issues

If your work is included here without proper attribution, please email
**legal@agentmux.ai** and we will fix it promptly.
