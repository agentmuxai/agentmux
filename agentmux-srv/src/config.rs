
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "agentmux-srv", about = "AgentMux Rust backend server")]
pub struct CliArgs {
    /// Path to wave data directory (overrides AGENTMUX_DATA_HOME)
    #[arg(long = "wavedata")]
    pub wavedata: Option<String>,

    /// Instance identifier (used for multi-version coexistence)
    #[arg(long = "instance", default_value = "default")]
    pub instance: String,

}

#[derive(Debug, Clone)]
pub struct Config {
    pub auth_key: String,
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

        // CLI flag wins over env. Env preference order is the new
        // unified `AGENTMUX_DATA_DIR` (set by the launcher via
        // `agentmux_common::DataPaths::to_env_vars`); the legacy
        // `AGENTMUX_DATA_HOME` name is no longer set.
        let data_home = args
            .wavedata
            .clone()
            .or_else(|| std::env::var("AGENTMUX_DATA_DIR").ok())
            .unwrap_or_default();

        // Same migration for config_home.
        let config_home = std::env::var("AGENTMUX_CONFIG_DIR")
            .or_else(|_| std::env::var("AGENTMUX_CONFIG_HOME"))
            .unwrap_or_default();
        let app_path = std::env::var("AGENTMUX_APP_PATH").unwrap_or_default();
        // is_dev is now derived from AGENTMUX_RUNTIME_MODE (the
        // canonical env var emitted by the unified DataPaths layer).
        // Legacy AGENTMUX_DEV is no longer set by the launcher.
        let is_dev = matches!(
            agentmux_common::RuntimeMode::from_env(),
            Some(agentmux_common::RuntimeMode::Dev { .. })
        );

        Ok(Config {
            auth_key,
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
        let args = CliArgs { wavedata: None, instance: "default".to_string() };
        let result = Config::from_env_and_args(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("AGENTMUX_AUTH_KEY"));
    }

    #[test]
    fn empty_auth_key_errors() {
        let _lock = lock();
        clear_env();
        std::env::set_var("AGENTMUX_AUTH_KEY", "");
        let args = CliArgs { wavedata: None, instance: "default".to_string() };
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
        let args = CliArgs { wavedata: None, instance: "default".to_string() };
        let config = Config::from_env_and_args(&args).unwrap();
        assert_eq!(config.data_home, "/data");
        assert_eq!(config.config_home, "/config");
        assert_eq!(config.app_path, "/app");
        assert!(config.is_dev);
        clear_env();
    }
}
