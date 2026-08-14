// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "agentmux-srv", about = "AgentMux Rust backend server")]
pub struct CliArgs {
    /// Path to wave data directory (overrides AGENTMUX_DATA_HOME)
    #[arg(long = "wavedata")]
    pub wavedata: Option<String>,

    /// Instance identifier (used for multi-version coexistence)
    #[arg(long = "instance", default_value = "default")]
    pub instance: String,

    #[command(subcommand)]
    pub command: Option<SrvCommand>,
}

#[derive(Subcommand, Debug)]
pub enum SrvCommand {
    /// Run pending data migrations and exit. Invoked by the launcher before
    /// starting the daemon so the srv always starts with clean migrated state.
    Migrate {
        /// Print pending migrations without applying them.
        #[arg(long)]
        dry_run: bool,
        /// List all migrations and their applied/pending status.
        #[arg(long)]
        list: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub auth_key: String,
    /// A separate, narrowly-scoped credential for LAN peer discovery
    /// (mDNS TXT record + UDP broadcast responder) — NOT `auth_key`.
    /// Minted fresh per-launch, internal to srv only (never needs to
    /// leave this process except via the LAN broadcast itself, unlike
    /// `auth_key` which the launcher/frontend also need — see
    /// `LanDiscovery`'s doc comment). Broadcasting the full-access
    /// `auth_key` to anything that can receive an mDNS multicast packet
    /// or send a UDP probe used to mean a passive LAN listener got
    /// standing access to the ENTIRE local `/agentmux/service` surface,
    /// not just LAN-forwarding — this key is accepted only by the two
    /// routes LAN peer forwarding actually needs
    /// (`lan_or_full_auth_middleware` in `server/mod.rs`), so a captured
    /// value's blast radius shrinks to "can forward jekts to this
    /// instance and query which agents live here." See
    /// docs/specs/SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md §2.1/§3
    /// LAN P0-1.
    pub lan_key: String,
    pub data_home: String,
    pub config_home: String,
    pub app_path: String,
    #[allow(dead_code)]
    pub is_dev: bool,
    pub version: &'static str,
    pub build_time: &'static str,
    pub instance_id: String,
}

impl Config {
    /// Build config from env vars + CLI args.
    /// Removes AGENTMUX_AUTH_KEY from the environment after reading (matching Go behavior).
    pub fn from_env_and_args(args: &CliArgs) -> Result<Self, String> {
        let auth_key = std::env::var("AGENTMUX_AUTH_KEY")
            .map_err(|_| "AGENTMUX_AUTH_KEY environment variable is required".to_string())?;

        if auth_key.is_empty() {
            return Err("AGENTMUX_AUTH_KEY must not be empty".to_string());
        }

        // Remove from env after read (matching Go authkey.go:50)
        std::env::remove_var("AGENTMUX_AUTH_KEY");

        // CLI flag wins over env. The launcher sets the canonical
        // `AGENTMUX_DATA_DIR` and `AGENTMUX_CONFIG_DIR` via
        // `agentmux_common::DataPaths::to_env_vars`. Pre-unification
        // names (`AGENTMUX_DATA_HOME`, `AGENTMUX_CONFIG_HOME`) are no
        // longer set — no fallback (symmetry; partial-rollout isn't a
        // supported scenario per spec §3.4 "no migration").
        let data_home = args
            .wavedata
            .clone()
            .or_else(|| std::env::var("AGENTMUX_DATA_DIR").ok())
            .unwrap_or_default();

        let config_home = std::env::var("AGENTMUX_CONFIG_DIR").unwrap_or_default();
        let app_path = std::env::var("AGENTMUX_APP_PATH").unwrap_or_default();
        // is_dev is now derived from AGENTMUX_RUNTIME_MODE (the
        // canonical env var emitted by the unified DataPaths layer).
        // Legacy AGENTMUX_DEV is no longer set by the launcher.
        let is_dev = matches!(
            agentmux_common::RuntimeMode::from_env(),
            Some(agentmux_common::RuntimeMode::Dev { .. })
        );

        // Two v4 UUIDs concatenated for margin over a single UUID's 122 bits
        // of randomness — same "avoid a rand/getrandom dependency just for
        // this" reasoning as agent_jekt_keys.rs's random_key_bytes(), but
        // kept as a plain string here (not raw bytes) since this only ever
        // travels as a String — an mDNS TXT record value / UDP JSON field /
        // HTTP header — never binary-serialized or HMAC-keyed.
        let lan_key = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

        Ok(Config {
            auth_key,
            lan_key,
            data_home,
            config_home,
            app_path,
            is_dev,
            version: env!("CARGO_PKG_VERSION"),
            build_time: option_env!("BUILD_TIME").unwrap_or("dev"),
            instance_id: args.instance.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize config tests — they mutate process-global env vars
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Lock helper that recovers from poisoned mutex. A panic in any
    /// test would otherwise propagate poison to all later tests via
    /// `lock().unwrap()` and produce noise unrelated to the actual
    /// failing test.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Clear every env var our config reads so each test starts from
    /// a known state, regardless of leakage from prior tests.
    fn clear_env() {
        for k in [
            "AGENTMUX_AUTH_KEY",
            "AGENTMUX_DATA_DIR",
            "AGENTMUX_DATA_HOME",
            "AGENTMUX_CONFIG_DIR",
            "AGENTMUX_CONFIG_HOME",
            "AGENTMUX_APP_PATH",
            "AGENTMUX_RUNTIME_MODE",
            "AGENTMUX_DEV",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn missing_auth_key_errors() {
        let _lock = lock();
        clear_env();
        let args = CliArgs { wavedata: None, instance: "default".to_string(), command: None };
        let result = Config::from_env_and_args(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("AGENTMUX_AUTH_KEY"));
    }

    #[test]
    fn empty_auth_key_errors() {
        let _lock = lock();
        clear_env();
        std::env::set_var("AGENTMUX_AUTH_KEY", "");
        let args = CliArgs { wavedata: None, instance: "default".to_string(), command: None };
        let result = Config::from_env_and_args(&args);
        assert!(result.is_err());
        clear_env();
    }

    #[test]
    fn cli_wavedata_overrides_env() {
        let _lock = lock();
        clear_env();
        std::env::set_var("AGENTMUX_AUTH_KEY", "test-key-12345");
        std::env::set_var("AGENTMUX_DATA_DIR", "/from/env");
        let args = CliArgs {
            wavedata: Some("/from/cli".to_string()),
            instance: "default".to_string(),
            command: None,
        };
        let config = Config::from_env_and_args(&args).unwrap();
        assert_eq!(config.data_home, "/from/cli");
        assert!(std::env::var("AGENTMUX_AUTH_KEY").is_err());
        clear_env();
    }

    #[test]
    fn env_var_parsing() {
        let _lock = lock();
        clear_env();
        std::env::set_var("AGENTMUX_AUTH_KEY", "test-key-67890");
        std::env::set_var("AGENTMUX_DATA_DIR", "/data");
        std::env::set_var("AGENTMUX_CONFIG_DIR", "/config");
        std::env::set_var("AGENTMUX_APP_PATH", "/app");
        std::env::set_var("AGENTMUX_RUNTIME_MODE", "dev:main");
        let args = CliArgs { wavedata: None, instance: "default".to_string(), command: None };
        let config = Config::from_env_and_args(&args).unwrap();
        assert_eq!(config.data_home, "/data");
        assert_eq!(config.config_home, "/config");
        assert_eq!(config.app_path, "/app");
        assert!(config.is_dev);
        clear_env();
    }

    #[test]
    fn lan_key_is_generated_nonempty_and_distinct_from_auth_key() {
        let _lock = lock();
        clear_env();
        std::env::set_var("AGENTMUX_AUTH_KEY", "test-key-67890");
        let args = CliArgs { wavedata: None, instance: "default".to_string(), command: None };
        let config = Config::from_env_and_args(&args).unwrap();
        assert!(!config.lan_key.is_empty());
        assert_ne!(
            config.lan_key, config.auth_key,
            "the LAN-broadcast credential must never equal the full-access auth_key"
        );
        clear_env();
    }

    #[test]
    fn lan_key_is_freshly_generated_per_call_not_a_fixed_constant() {
        let _lock = lock();
        clear_env();
        std::env::set_var("AGENTMUX_AUTH_KEY", "test-key-67890");
        let args = CliArgs { wavedata: None, instance: "default".to_string(), command: None };
        let first = Config::from_env_and_args(&args).unwrap().lan_key;
        std::env::set_var("AGENTMUX_AUTH_KEY", "test-key-67890");
        let second = Config::from_env_and_args(&args).unwrap().lan_key;
        assert_ne!(first, second, "each process launch must mint its own lan_key");
        clear_env();
    }
}
