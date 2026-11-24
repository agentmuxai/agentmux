// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The window manager's settings. A pane's client sends keys and mouse buttons at screen level, so whatever the window manager binds to a
//! key or a button, the client can use: a stock Openbox has a root menu that starts programs on the server and an Alt-drag that moves a
//! window. This configuration binds nothing: no keyboard shortcut, no mouse action, an empty menu, and no frame on any window (so there is
//! no title bar to drag and no button to press). The window manager is there to give the browser the focus and to place its windows; the
//! browser is the only program a client can reach.

use std::path::Path;

/// The window manager's own file names, inside the folder it is given.
pub const RC_FILE: &str = "rc.xml";
pub const MENU_FILE: &str = "menu.xml";

/// The settings: nothing bound, the menu empty, no window decorated.
pub fn rc(menu: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<openbox_config xmlns="http://openbox.org/3.4/rc">
  <focus><focusNew>yes</focusNew><followMouse>no</followMouse><focusLast>yes</focusLast></focus>
  <placement><policy>Smart</policy></placement>
  <desktops><number>1</number></desktops>
  <keyboard></keyboard>
  <mouse><dragThreshold>8</dragThreshold><doubleClickTime>500</doubleClickTime></mouse>
  <menu><file>{}</file></menu>
  <applications><application class="*"><decor>no</decor></application></applications>
</openbox_config>
"#,
        menu.display()
    )
}

/// A menu with nothing in it.
pub const MENU: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<openbox_menu xmlns="http://openbox.org/3.4/menu">
  <menu id="root-menu" label="AgentMux"></menu>
</openbox_menu>
"#;

/// Write both files into `dir` and give the arguments that make the window manager read them.
pub fn write(dir: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the window manager's folder: {e}"))?;
    let (rc_path, menu_path) = (dir.join(RC_FILE), dir.join(MENU_FILE));
    std::fs::write(&menu_path, MENU).map_err(|e| format!("cannot write the window manager's menu: {e}"))?;
    std::fs::write(&rc_path, rc(&menu_path)).map_err(|e| format!("cannot write the window manager's settings: {e}"))?;
    Ok(vec!["--config-file".to_string(), rc_path.display().to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_manager_binds_nothing_a_client_could_use_to_run_a_program_or_move_a_window() {
        let text = rc(Path::new("/tmp/wm/menu.xml"));
        for bound in ["<keybind", "<mousebind", "<action", "Execute", "ShowMenu", "Move"] {
            assert!(!text.contains(bound), "{bound} must not be in the settings");
        }
        assert!(text.contains("<keyboard></keyboard>"), "the keyboard section is there and empty (an absent one may fall back to defaults)");
        assert!(text.contains("<decor>no</decor>"), "no window has a frame to drag");
        assert!(text.contains("/tmp/wm/menu.xml"));
        assert!(!MENU.contains("<item") && !MENU.contains("Execute"), "the menu is empty");
    }

    #[test]
    fn the_files_are_written_and_the_arguments_name_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let args = write(&dir.path().join("wm")).unwrap();
        assert_eq!(args[0], "--config-file");
        assert!(Path::new(&args[1]).is_file() && dir.path().join("wm").join(MENU_FILE).is_file());
    }
}
