// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps that run as an administrator, for `agentmux service install / uninstall / start / stop`
//! (docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md, B.1, B.3, C and E). The person's own command reads and holds the
//! package, then runs this step elevated; this step reads the same package again and must find the digest it was given, makes the
//! service, claims the folders, copies and verifies the files, and starts it. Everything it says is also written to
//! `last-run.log` in the program folder (only administrators write there), which the person's command prints afterwards: the
//! elevated window closes when it ends.

use std::io::Write;
use std::path::{Path, PathBuf};

use agentmux_common::owner_only;
use windows_service::service::{Service, ServiceAccess};

use super::winsvc_cli::{Action, ApplyArgs};
use super::winsvc_files as files;
use super::winsvc_plan::{self as plan, Layout, Reach};
use super::winsvc_win::{self as win, Known};
use super::winservice;

/// The seconds a start or a stop may take before it is called stuck.
const WAIT_SECS: u64 = 60;

/// What the step says, to its window and to the log file the person's command reads.
struct Report {
    file: Option<std::fs::File>,
}

impl Report {
    fn say(&mut self, line: &str) {
        println!("{line}");
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Start the log, replacing the last run's.
    fn open(&mut self, log: &Path) {
        if self.file.is_none() {
            self.file = std::fs::File::create(log).ok();
        }
    }
}

/// The log the step writes for a channel (only administrators can write there; everyone can read it).
pub fn log_path_of(channel: &str) -> Result<PathBuf, String> {
    let layout = plan::layout(&win::known_folder(Known::ProgramFiles)?, &win::known_folder(Known::ProgramData)?, channel, "0");
    layout.program_root.parent().map(|p| p.join(plan::log_name(channel))).ok_or_else(|| "cannot tell where the log goes".to_string())
}

/// `agentmux service apply <action>`: the step that needs an administrator. Returns the exit code.
pub fn run(channel: &str, a: &ApplyArgs) -> i32 {
    let mut report = Report { file: None };
    let outcome = run_inner(channel, a, &mut report);
    let code = match &outcome {
        Ok(()) => 0,
        Err(e) => {
            report.say(&format!("agentmux: {e}"));
            3
        }
    };
    // A window nobody reads is closed at once; one whose words reached no log is held open, or they would be lost.
    if a.pause && code != 0 && report.file.is_none() {
        println!("Press Enter to close this window.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    code
}

fn run_inner(channel: &str, a: &ApplyArgs, report: &mut Report) -> Result<(), String> {
    if !owner_only::is_elevated() {
        return Err(plan::NEEDS_ADMINISTRATOR.to_string());
    }
    let (files_dir, data_dir) = (win::known_folder(Known::ProgramFiles)?, win::known_folder(Known::ProgramData)?);
    let layout = plan::layout(&files_dir, &data_dir, channel, crate::VERSION);
    // The folder above the program folder is made first and checked, so the log has a place whatever fails next.
    if let (Some(parent), Ok(log)) = (layout.program_root.parent(), log_path_of(channel)) {
        if parent.is_dir() || a.action == Action::Install {
            let _ = claim(parent, &plan::parent_sddl(), &[], Reach::Above, false);
            report.open(&log);
        }
    }
    match a.action {
        Action::Install => install(channel, a, &layout, report),
        other => {
            only_the_installed_copy(&layout)?;
            let service = win::open(&winservice::service_name(channel), ServiceAccess::ALL_ACCESS).map_err(|_| "the service is not installed".to_string())?;
            manage(other, a, &service, &layout, report)
        }
    }
}

/// Stop, start or remove a service that is there.
fn manage(action: Action, a: &ApplyArgs, service: &Service, layout: &Layout, report: &mut Report) -> Result<(), String> {
    match action {
        Action::Start => win::start(service, WAIT_SECS).map(|_| report.say("The service is running.")),
        Action::Stop => win::stop(service, WAIT_SECS).map(|_| report.say("The service is stopped.")),
        Action::Uninstall => {
            win::stop(service, WAIT_SECS)?;
            service.delete().map_err(|e| format!("cannot remove the service: {e}"))?;
            report.say("The service is removed. Its program folder and its data are kept.");
            if a.purge {
                if std::fs::symlink_metadata(&layout.data_root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
                    report.say(&format!("{} is already gone: nothing to delete.", layout.data_root.display()));
                    return Ok(());
                }
                purge(&layout.data_root).map_err(|e| format!("cannot remove {}: {e}", layout.data_root.display()))?;
                report.say(&format!("{} is deleted: the service's data, its keys and its sign-ins are gone.", layout.data_root.display()));
            }
            Ok(())
        }
        Action::Install => unreachable!("handled by the caller"),
    }
}

/// Delete a folder and everything in it, including what the service made for its own account alone: each folder and file is first
/// taken over by the administrators (the step runs elevated), and a link is removed as a link, never followed. The take-over is
/// made through a handle on the entry itself, which says whether it is a link, so an entry the service's account swapped for a link
/// is never followed; once a folder is taken over that account cannot change it, so what is read from it next is what was taken.
fn purge(path: &Path) -> Result<(), String> {
    let taken = owner_only::take_over(path)?;
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let is_link = !taken || meta.file_type().is_symlink() || std::os::windows::fs::MetadataExt::file_attributes(&meta) & 0x400 != 0;
    if is_link {
        return std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path)).map_err(|e| e.to_string());
    }
    if meta.is_dir() {
        for item in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            purge(&item.map_err(|e| e.to_string())?.path())?;
        }
        return std::fs::remove_dir(path).map_err(|e| e.to_string());
    }
    let mut readonly = meta.permissions();
    readonly.set_readonly(false);
    let _ = std::fs::set_permissions(path, readonly);
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// An administrator step other than the install runs only the copy in the program folder, which only administrators write:
/// never a program a person's account could have replaced.
fn only_the_installed_copy(layout: &Layout) -> Result<(), String> {
    let me = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot find this program: {e}"))?;
    let root = layout.program_root.canonicalize().map_err(|_| "the service is not installed".to_string())?;
    if me.starts_with(&root) {
        Ok(())
    } else {
        Err(format!("this step runs only the installed copy of AgentMux Headless (in {}), not {}", root.display(), me.display()))
    }
}

/// The folder the package this program is in starts at: its `bin`'s parent.
fn package_root() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot find this program: {e}"))?;
    exe.parent().and_then(Path::parent).map(Path::to_path_buf).ok_or_else(|| "cannot find the package this program is in".to_string())
}

/// The accounts that may read the key, as given: each must be a person's own account, as Windows says too, or the install is
/// refused.
fn readers_of(a: &ApplyArgs) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for sid in &a.reader {
        if let Some(problem) = plan::reader_problem(sid) {
            return Err(problem);
        }
        if !owner_only::sid_is_user(sid) {
            return Err(format!("{sid} is not a user account on this computer (Windows says it is a group or unknown). Pass the SID `whoami /user` gives for the account that signs in"));
        }
        out.push(sid.clone());
    }
    Ok(out)
}

/// `service install`, as an administrator. Everything that can refuse comes first and leaves a service that is installed as it
/// was: the arguments, the package and its digest, the folders and the staged copy, hashed at its destination. Only then is the
/// service touched.
fn install(channel: &str, a: &ApplyArgs, layout: &Layout, report: &mut Report) -> Result<(), String> {
    if !plan::version_is_valid(crate::VERSION) {
        return Err(format!("{:?} is not a version a folder can be named for", crate::VERSION));
    }
    if !a.allow_domain_joined && win::domain_joined() {
        return Err(plan::DOMAIN_JOINED.to_string());
    }
    let readers = readers_of(a)?;
    let package = files::read_package(&package_root()?, true)?;
    let digest = package.digest();
    if let Some(problem) = plan::digest_problem(a.digest.as_deref(), &digest) {
        return Err(problem);
    }
    check_folders(layout)?;
    report.say(&format!("Copying AgentMux Headless {} to a staging folder and checking it.", crate::VERSION));
    let stage = stage_copy(layout, &package, &digest)?;
    let done = commit(channel, a, layout, stage.as_deref(), &readers, report);
    if let Some(stage) = &stage {
        let _ = std::fs::remove_dir_all(stage);
    }
    done
}

/// Touch the service: stop the one that is there, make or change it, give it its folders and its version, and start it. If this
/// fails after the old service was stopped, it is started again (the service it was, or the one this made is removed).
fn commit(channel: &str, a: &ApplyArgs, layout: &Layout, stage: Option<&Path>, readers: &[String], report: &mut Report) -> Result<(), String> {
    let name = winservice::service_name(channel);
    let old = win::open(&name, ServiceAccess::ALL_ACCESS).ok();
    if let Some(old) = &old {
        report.say("Stopping the service that is installed.");
        win::stop(old, WAIT_SECS)?;
    }
    // The service first: its account's SID is what the folders' access lists name.
    let (service, made) = match win::create_or_update(channel, &layout.exe, &layout.data_root, a.port) {
        Ok(done) => done,
        Err(e) => {
            if let Some(old) = &old {
                let _ = win::start(old, WAIT_SECS);
            }
            return Err(e);
        }
    };
    let done = commit_into(channel, layout, stage, readers, &service, report);
    if done.is_err() {
        if made {
            let _ = service.delete();
        } else {
            let _ = win::start(&service, WAIT_SECS);
        }
    }
    done
}

fn commit_into(channel: &str, layout: &Layout, stage: Option<&Path>, readers: &[String], service: &Service, report: &mut Report) -> Result<(), String> {
    win::finish_definition(service)?;
    let sid = owner_only::account_sid(&winservice::virtual_account(channel))?;
    if !readers.is_empty() {
        win::set_service_environment(&winservice::service_name(channel), &[format!("{}={}", owner_only::KEY_READERS_ENV, readers.join(","))])?;
    }
    claim_folders(layout, &sid)?;
    if let Some(stage) = stage {
        place_version(layout, stage)?;
    }
    agentmux_common::data_paths::ensure_history_link(&layout.current, &layout.version_dir).map_err(|e| format!("cannot point {} at the new version: {e}", layout.current.display()))?;
    report.say("Starting the service.");
    win::start(service, WAIT_SECS).map_err(|e| format!("{e}. The service stays installed: `agentmux service status` says how it is, and its log is in {}", layout.data_root.display()))?;
    for line in plan::after_install(&winservice::service_name(channel), &layout.exe, &layout.data_root) {
        report.say(&line);
    }
    Ok(())
}

/// Every check that needs no service: the folders above, and a program folder or data root that is already there (it must be
/// owned by an administrator and be one nobody else can change, whichever service's account it names).
fn check_folders(layout: &Layout) -> Result<(), String> {
    for root in [&layout.program_root, &layout.data_root] {
        if let Some(parent) = root.parent() {
            claim(parent, &plan::parent_sddl(), &[], Reach::Above, false)?;
            check_above(parent)?;
        }
        if root.exists() {
            if let Some(problem) = plan::folder_problem(root, &owner_only::owner_and_sddl_of(root)?, &["S-1-5-80-*"], Reach::TheFolder) {
                return Err(problem);
            }
        }
    }
    Ok(())
}

/// Copy the package into a staging folder beside the program folder (which only administrators can write) and hash it there: it
/// must give the package's digest. `None` when this program already runs from the version folder it would install.
fn stage_copy(layout: &Layout, package: &files::Package, digest: &str) -> Result<Option<PathBuf>, String> {
    if package.root.canonicalize().ok() == layout.version_dir.canonicalize().ok() {
        return Ok(None);
    }
    let parent = layout.program_root.parent().ok_or_else(|| "cannot tell where the program folder is".to_string())?;
    let stage = parent.join(format!(".stage-{}", uuid::Uuid::new_v4().simple()));
    let copied = files::copy_package(package, &stage).and_then(|_| files::verify_copy(&stage, digest));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    Ok(Some(stage))
}

/// Make the folders the service uses, each with its own access list from the moment it exists, or take over one that is
/// there only when an administrator owns it and it is fit.
fn claim_folders(layout: &Layout, service_sid: &str) -> Result<(), String> {
    claim(&layout.program_root, &plan::program_sddl(service_sid), &[], Reach::TheFolder, true)?;
    claim(&layout.data_root, &plan::data_sddl(service_sid), &[service_sid], Reach::TheFolder, true)
}

/// `own` says the folder is this service's own: a folder that is there already is given the list too. One that is not (the
/// `AgentMux` folder above, which a desktop install may also use) is only checked.
fn claim(folder: &Path, sddl: &str, allowed: &[&str], reach: Reach, own: bool) -> Result<(), String> {
    match owner_only::create_dir_with_sddl(folder, sddl) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if let Some(problem) = plan::folder_problem(folder, &owner_only::owner_and_sddl_of(folder)?, allowed, reach) {
                return Err(problem);
            }
            if own {
                owner_only::apply_sddl(folder, sddl)?;
            }
        }
        Err(e) => return Err(format!("cannot make {}: {e}", folder.display())),
    }
    match plan::folder_problem(folder, &owner_only::owner_and_sddl_of(folder)?, allowed, reach) {
        Some(problem) => Err(problem),
        None => Ok(()),
    }
}

/// No folder above `folder`, up to the root of its drive, may be one that a person who is not an administrator can change
/// or replace what is below (creating an entry beside it is not that).
fn check_above(folder: &Path) -> Result<(), String> {
    if skip_requested() {
        return Ok(());
    }
    for above in folder.ancestors().skip(1) {
        let who = plan::who_can_change(&owner_only::owner_and_sddl_of(above)?, &[], Reach::Above);
        if !who.is_empty() {
            return Err(plan::above_problem(above, &who));
        }
    }
    Ok(())
}

/// A test run of a debug build can skip the check of the folders above (and only that one). The release build does not contain
/// the code, or the name of the variable.
#[cfg(debug_assertions)]
fn skip_requested() -> bool {
    std::env::var_os("AGENTMUX_TEST_SKIP_ABOVE_CHECK").is_some()
}

#[cfg(not(debug_assertions))]
fn skip_requested() -> bool {
    false
}

/// Move the staged package into its version folder. A version that is already there is moved aside first and removed once the new
/// one is in.
fn place_version(layout: &Layout, stage: &Path) -> Result<(), String> {
    let aside = layout.program_root.join(format!(".old-{}", uuid::Uuid::new_v4().simple()));
    let moved_aside = layout.version_dir.exists();
    if moved_aside {
        std::fs::rename(&layout.version_dir, &aside).map_err(|e| format!("cannot replace {}: {e}", layout.version_dir.display()))?;
    }
    if let Err(e) = std::fs::rename(stage, &layout.version_dir) {
        if moved_aside {
            let _ = std::fs::rename(&aside, &layout.version_dir);
        }
        return Err(format!("cannot move the copy into {}: {e}", layout.version_dir.display()));
    }
    if moved_aside {
        let _ = std::fs::remove_dir_all(&aside);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs an elevated token (the take-over uses the right to take ownership), so it does nothing in a terminal that is not.
    #[test]
    fn purge_deletes_a_tree_and_never_follows_a_link_out_of_it() {
        if !owner_only::is_elevated() {
            return;
        }
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.txt"), "kept").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("data");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/key"), "k").unwrap();
        std::fs::write(root.join("top.txt"), "t").unwrap();
        agentmux_common::data_paths::ensure_history_link(&root.join("a/out"), outside.path()).unwrap();
        purge(&root).unwrap();
        assert!(!root.exists(), "the data root is gone");
        assert_eq!(std::fs::read_to_string(outside.path().join("keep.txt")).unwrap(), "kept", "what the link pointed to is untouched");
        assert!(owner_only::take_over(&root).is_err(), "a missing entry is an error, not a success");
    }
}
