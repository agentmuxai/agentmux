// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fresh data directory for one test, shared by the tests that need `DataPaths::from_env()`.

/// Points the data-dir variables at a fresh folder for one test and puts them back on drop.
/// Hold `ISOLATED_AUTH_ENV_LOCK` for as long as it lives.
pub(crate) struct DataEnv {
    _dir: tempfile::TempDir,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl DataEnv {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let instance = root.join("channels").join("c");
        let version = instance.join("versions").join("1.0.0");
        let paths = agentmux_common::DataPaths {
            home_dir: version.join("data"),
            instance_dir: instance.clone(),
            channel: "c".to_string(),
            data_dir: version.join("data"),
            config_dir: instance.join("config"),
            logs_dir: version.join("logs"),
            cef_cache_dir: version.join("cef-cache"),
            agents_dir: instance.join("agents"),
            instance_runtime_dir: version.join("runtime"),
            shared_dir: root.join("shared"),
            mode: agentmux_common::RuntimeMode::Portable,
        };
        let mut vars = paths.to_env_vars();
        vars.push(("AGENTMUX_HOME_OVERRIDE", root.join("home").into_os_string()));
        let mut saved: Vec<_> = vars.iter().map(|(k, _)| (*k, std::env::var_os(k))).collect();
        saved.push(("AGENTMUX_CLONE_ID", std::env::var_os("AGENTMUX_CLONE_ID")));
        std::env::remove_var("AGENTMUX_CLONE_ID");
        for (k, v) in &vars {
            std::env::set_var(k, v);
        }
        DataEnv { _dir: dir, saved }
    }
}

impl Drop for DataEnv {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}
