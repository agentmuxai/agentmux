# CEF & Chromium Version Report — 2026-04-03

## Current AgentMux

| Component | Version |
|-----------|---------|
| CEF Rust crate | `cef 146.4.0+146.0.9` (pinned `146` in Cargo.toml) |
| CEF upstream | 146.0.9 (`146.0.9+g3ca6a87+chromium-146.0.7680.165`) |
| Chromium (bundled) | 146.0.7680.165 |

## Latest Available

| Component | Version | Notes |
|-----------|---------|-------|
| CEF stable | **146.0.9** | Latest on Spotify CDN |
| Chrome stable | **147.0.7727.49** | Released 2026-04-01 (early stable) |
| CEF 147 branch | Not yet stable | No stable build on CDN yet |

## Assessment

- AgentMux is **on the latest stable CEF** (146.0.9 / Chromium 146)
- One Chromium milestone behind Chrome stable (146 vs 147) — this is normal, CEF trails by ~1 milestone
- No action needed until CEF 147 has a stable release on the Spotify CDN
- When CEF 147 ships: update `cef` version pin in `agentmux-cef/Cargo.toml`, rebuild, and test
