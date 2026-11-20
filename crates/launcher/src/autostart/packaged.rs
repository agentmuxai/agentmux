//! Start at login for the Microsoft Store (MSIX) package (`SPEC_START_WITH_OS_2026_09_25.md` §3.7).
//!
//! A packaged app cannot rely on a Scheduled Task that points into its versioned install folder, and the
//! person manages its login entry in Windows (Settings > Apps > Startup, Task Manager > Startup apps), so the
//! package declares a `windows.startupTask` (`packaging/msix/AppxManifest.xml.template`) and this module is the
//! one place that talks to it. Removing the package removes the task.
//!
//! Windows owns the state, and the person can change it outside AgentMux, so it is read every time and never
//! cached: [`TaskState`] is what Windows says now.

use std::sync::OnceLock;

use windows::{
    core::HSTRING,
    ApplicationModel::{Activation::ActivationKind, AppInstance, StartupTask, StartupTaskState},
    Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
};

/// The manifest's `StartupTask` `TaskId`. One per package: the entry carries no channel, because a package
/// is one channel's install.
pub const TASK_ID: &str = "AgentMuxStartup";

/// `APPMODEL_ERROR_NO_PACKAGE`: the process has no package identity.
const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentPackageFullName(length: *mut u32, name: *mut u16) -> i32;
}

/// Does this process run with package identity (installed from the Store or a sideloaded MSIX)?
pub fn is_packaged() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut len = 0u32;
        // A packaged process answers ERROR_INSUFFICIENT_BUFFER for a zero-length buffer; an unpackaged one
        // answers APPMODEL_ERROR_NO_PACKAGE.
        let rc = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
        rc != APPMODEL_ERROR_NO_PACKAGE
    })
}

/// What Windows says about the package's startup task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Off; AgentMux may ask to turn it on.
    Disabled,
    /// The person turned it off in Windows. AgentMux cannot turn it back on; they must.
    DisabledByUser,
    /// On.
    Enabled,
    /// Off by group policy; nothing in AgentMux can change it.
    DisabledByPolicy,
    /// On by group policy.
    EnabledByPolicy,
}

impl TaskState {
    fn from_winrt(s: StartupTaskState) -> Self {
        match s {
            StartupTaskState::DisabledByUser => Self::DisabledByUser,
            StartupTaskState::Enabled => Self::Enabled,
            StartupTaskState::DisabledByPolicy => Self::DisabledByPolicy,
            StartupTaskState::EnabledByPolicy => Self::EnabledByPolicy,
            _ => Self::Disabled,
        }
    }

    /// The name Settings and `--autostart-status --json` use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::DisabledByUser => "disabled_by_user",
            Self::Enabled => "enabled",
            Self::DisabledByPolicy => "disabled_by_policy",
            Self::EnabledByPolicy => "enabled_by_policy",
        }
    }

    /// Windows will start the app at login.
    pub fn is_on(self) -> bool {
        matches!(self, Self::Enabled | Self::EnabledByPolicy)
    }
}

/// WinRT needs an apartment on this thread; the launcher's CLI verbs run before anything else has made one.
/// Already initialised (to either kind) is fine.
fn ensure_winrt() {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

fn task() -> Result<StartupTask, String> {
    ensure_winrt();
    StartupTask::GetAsync(&HSTRING::from(TASK_ID))
        .and_then(|op| op.get())
        .map_err(|e| format!("the package's startup task: {e}"))
}

/// The task's state now. `Err` when Windows cannot say (not packaged, or the manifest lacks the task).
pub fn state() -> Result<TaskState, String> {
    let t = task()?;
    t.State().map(TaskState::from_winrt).map_err(|e| format!("read the startup task's state: {e}"))
}

/// Ask Windows to turn the task on. The answer is the state afterwards: a person's earlier "off" in Windows
/// stays off, and the caller says so rather than pretending.
pub fn request_enable() -> Result<TaskState, String> {
    let t = task()?;
    t.RequestEnableAsync()
        .and_then(|op| op.get())
        .map(TaskState::from_winrt)
        .map_err(|e| format!("turn the startup task on: {e}"))
}

/// Turn the task off. Windows keeps it off until the person or AgentMux turns it on again.
pub fn disable() -> Result<(), String> {
    task()?.Disable().map_err(|e| format!("turn the startup task off: {e}"))
}

/// Did Windows start this process because the person signed in (the package's startup task), not because they
/// opened AgentMux? A startup task starts the exe with no arguments, so the activation kind is the only mark.
/// Asked once: it describes how this process was started.
pub fn started_by_startup_task() -> bool {
    static BY_TASK: OnceLock<bool> = OnceLock::new();
    *BY_TASK.get_or_init(|| activation().map(|k| k == ActivationKind::StartupTask).unwrap_or(false))
}

/// How Windows activated this process, or why it cannot say. Only a packaged process has an answer.
fn activation() -> Result<ActivationKind, String> {
    if !is_packaged() {
        return Err("not packaged".to_string());
    }
    ensure_winrt();
    AppInstance::GetActivatedEventArgs().and_then(|a| a.Kind()).map_err(|e| e.to_string())
}

/// [`activation`] as a word for `--autostart-status --json`, so a bug report says whether a login start was
/// recognised: `startup_task` is the person signing in, `launch` an ordinary open, anything else is Windows'
/// own name for it, and `unknown` means the question failed.
pub fn activation_name() -> String {
    match activation() {
        Ok(ActivationKind::StartupTask) => "startup_task".to_string(),
        Ok(ActivationKind::Launch) => "launch".to_string(),
        Ok(other) => format!("{other:?}").to_lowercase(),
        Err(_) => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_enabled_states_start_at_login() {
        assert!(TaskState::Enabled.is_on());
        assert!(TaskState::EnabledByPolicy.is_on());
        assert!(!TaskState::Disabled.is_on());
        assert!(!TaskState::DisabledByUser.is_on());
        assert!(!TaskState::DisabledByPolicy.is_on());
    }

    /// Settings and `--autostart-status --json` print these words, and the Settings row keys off them.
    #[test]
    fn state_names_are_what_the_status_prints() {
        assert_eq!(TaskState::from_winrt(StartupTaskState::Disabled).as_str(), "disabled");
        assert_eq!(TaskState::from_winrt(StartupTaskState::DisabledByUser).as_str(), "disabled_by_user");
        assert_eq!(TaskState::from_winrt(StartupTaskState::Enabled).as_str(), "enabled");
        assert_eq!(TaskState::from_winrt(StartupTaskState::DisabledByPolicy).as_str(), "disabled_by_policy");
        assert_eq!(TaskState::from_winrt(StartupTaskState::EnabledByPolicy).as_str(), "enabled_by_policy");
    }

    /// The test binary has no package identity: nothing here may call into the package's startup task.
    #[test]
    fn an_unpackaged_process_is_not_a_login_start() {
        assert!(!is_packaged());
        assert!(!started_by_startup_task());
        assert_eq!(activation_name(), "unknown");
    }
}
