//! The Windows Task Scheduler registration XML for the login entry (the Scheduled Task every install but the Store
//! package uses; the package's own entry is `packaged.rs`). Pure, so the XML is tested on every platform.

use super::{xml_escape, AUTOSTART_ID, TASK_CHANNEL_MARKER};
use std::path::Path;

/// Windows Task Scheduler registration XML.
///
/// `LogonType=InteractiveToken` is the load-bearing detail (issue WS2): it
/// runs the task as the logged-in user *in their interactive session*, which
/// is what lets the process own a tray icon and later spawn visible windows.
/// `RunLevel=LeastPrivilege` keeps it out of elevation prompts — auto-start
/// must not require admin.
///
/// `<Delay>PT30S</Delay>` staggers startup behind the desktop's own login
/// work; a background service racing Explorer for disk on every boot is a
/// bad citizen, and 30s is invisible for something meant to run all day.
///
/// `DisallowStartIfOnBatteries`/`StopIfGoingOnBatteries` are both false
/// deliberately: the defaults are *true*, which would make a laptop user's
/// background service silently not start, or die mid-turn on unplug — the
/// exact "is it actually running?" unreliability WS4 warns about.
#[cfg(any(target_os = "windows", test))]
pub fn scheduled_task_xml(exe: &Path, channel: &str) -> String {
    scheduled_task_xml_for(exe, channel, current_user_id().as_deref())
}

/// The signed-in user as `DOMAIN\user`, the form Task Scheduler takes in `<UserId>`.
#[cfg(any(target_os = "windows", test))]
fn current_user_id() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    Some(match std::env::var("USERDOMAIN").ok().filter(|d| !d.is_empty()) {
        Some(domain) => format!("{domain}\\{user}"),
        None => user,
    })
}

/// [`scheduled_task_xml`] for a named user. A logon trigger with no `<UserId>` means "any user who signs in":
/// Windows lets only an elevated token create that (`schtasks /Create` fails with "Access is denied" from a
/// normal one), and it would start AgentMux for every other account on the machine too. With the user named, the
/// trigger is this user's alone and needs no elevation. `None` keeps the any-user form.
#[cfg(any(target_os = "windows", test))]
pub fn scheduled_task_xml_for(exe: &Path, channel: &str, user: Option<&str>) -> String {
    let exe = xml_escape(&exe.display().to_string());
    let channel = xml_escape(channel);
    let user_id = user.map(|u| format!("\n      <UserId>{}</UserId>", xml_escape(u))).unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts AgentMux in the background at logon. Created by AgentMux; remove it by turning off auto-start in AgentMux, or with: schtasks /Delete /TN "{id}" /F {marker}{channel}]</Description>
    <URI>\{id}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>{user_id}
      <Delay>PT30S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">{user_id}
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>false</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>--background</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        id = AUTOSTART_ID,
        exe = exe,
        marker = TASK_CHANNEL_MARKER,
        channel = channel,
        user_id = user_id,
    )
}
