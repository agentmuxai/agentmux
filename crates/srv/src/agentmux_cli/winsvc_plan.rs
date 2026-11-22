// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the installer of AgentMux Headless as a Windows service decides, with nothing in it that touches Windows
//! (docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md, B.1, B.3, C and E): where the files go, the access lists they
//! are given, who may change a folder the service's program or data live in, what a package must hold, the digest that says
//! the copy is the package, and the words a refusal uses. The effects (the service manager, the copy, the elevation) are in
//! `winsvc_win`; this part is the same code, and the same tests, on every platform.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use agentmux_common::owner_only::say_who;
use sha2::{Digest, Sha256};

/// Where everything of one channel's service goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `%ProgramFiles%\AgentMux\<name>`: only administrators write it. Holds a folder per version and the `current` link.
    pub program_root: PathBuf,
    /// `<program_root>\<version>`.
    pub version_dir: PathBuf,
    /// `<program_root>\current`: a junction to the version in use.
    pub current: PathBuf,
    /// What the service manager runs: `<current>\bin\agentmux.exe`.
    pub exe: PathBuf,
    /// `%ProgramData%\AgentMux\<name>`: the service's own folder, its data root.
    pub data_root: PathBuf,
}

/// The folder name of a channel's service under `AgentMux`: `Headless`, and `Headless-<channel>` for any other channel.
pub fn folder_name(channel: &str) -> String {
    if channel == crate::DEFAULT_CHANNEL { "Headless".to_string() } else { format!("Headless-{channel}") }
}

/// The log the administrator step writes for a channel, in the `AgentMux` folder above the program folder (which is there
/// before the program folder is, so even a step that fails at the first check leaves its words).
pub fn log_name(channel: &str) -> String {
    format!("{}-last-run.log", folder_name(channel))
}

/// The places for `channel`'s service and the `version` installed, from the two system folders (never from the
/// environment of whoever runs this: the caller asks Windows for them).
pub fn layout(program_files: &Path, program_data: &Path, channel: &str, version: &str) -> Layout {
    let name = folder_name(channel);
    let program_root = program_files.join("AgentMux").join(&name);
    let current = program_root.join("current");
    Layout {
        version_dir: program_root.join(version),
        exe: current.join("bin").join("agentmux.exe"),
        current,
        program_root,
        data_root: program_data.join("AgentMux").join(name),
    }
}

/// What the service manager is given after the program: the channel, the data root, the `service run` entry, and the web port
/// when one was asked for (the saved settings, else 7380, otherwise). Each is one argument; the manager quotes them.
pub fn launch_arguments(channel: &str, home: &Path, port: Option<u16>) -> Vec<String> {
    let mut args: Vec<String> = ["--channel", channel, "--home"].iter().map(|s| s.to_string()).collect();
    args.push(home.to_string_lossy().into_owned());
    args.extend(["service".to_string(), "run".to_string()]);
    if let Some(port) = port {
        args.extend(["--port".to_string(), port.to_string()]);
    }
    args
}

/// The server's key file for a version, in the data root of a service: where the server keeps its own, inside the channel's
/// folder (`DataPaths`: `<root>/channels/<channel>/versions/<version>/data`).
pub fn key_file(data_root: &Path, channel: &str, version: &str) -> PathBuf {
    data_root.join("channels").join(channel).join("versions").join(version).join("data").join("srv-auth-key")
}

/// The file the server writes its web port to, beside its key.
pub fn port_file(key_file: &Path) -> PathBuf {
    let mut name = key_file.file_name().unwrap_or_default().to_os_string();
    name.push(".port");
    key_file.with_file_name(name)
}

/// A version that is one folder name: digits and dots, with the pre-release part the releases use (`0.60.38`, `0.60.38-rc.1`).
pub fn version_is_valid(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version.starts_with(|c: char| c.is_ascii_digit())
        && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        && !version.contains("..")
}

/// The system and the administrators, in full, for a folder and everything made in it.
const ADMIN_FULL: &str = "(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
/// Read, execute and traverse: what a program folder gives the people who may run what is in it.
const READ_EXECUTE: &str = "0x1200a9";

/// The access list of the program folder (and so of each version in it): the system and the administrators in full, the
/// service's account and the users of this computer read-and-execute, nothing inherited from above and nobody else (D5).
pub fn program_sddl(service_sid: &str) -> String {
    format!("D:P{ADMIN_FULL}(A;OICI;{READ_EXECUTE};;;{service_sid})(A;OICI;{READ_EXECUTE};;;BU)")
}

/// The access list of the service's data root: its own account, the system and the administrators in full. Not the person
/// who installed it (they do not need to read API keys or sessions), and nobody else (B.3).
pub fn data_sddl(service_sid: &str) -> String {
    format!("D:P(A;OICI;FA;;;{service_sid}){ADMIN_FULL}")
}

/// The access list of the `AgentMux` folder both roots are in: the system and the administrators in full, the users of
/// this computer read-and-traverse, so a new folder of someone else cannot be made inside it, and neither root can be
/// renamed or deleted by a person who can only create entries in the folder above.
pub fn parent_sddl() -> String {
    format!("D:P{ADMIN_FULL}(A;OICI;{READ_EXECUTE};;;BU)")
}

/// The principals that are the administrators of the computer, as aliases and as SIDs; TrustedInstaller owns most of
/// `%ProgramFiles%` and is one of them.
const ADMINS: [&str; 6] = ["SY", "BA", "S-1-5-18", "S-1-5-32-544", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464", "CO"];

/// Which rights count as changing the folder. For the folder a service runs from or keeps its data in, any write counts. For a
/// folder above it, creating an entry beside it does not (`%ProgramData%` lets every user do that); only what would let
/// someone replace what is below does: delete a child, delete the folder, change its list or its owner, and every right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The folder is the one the service uses.
    TheFolder,
    /// The folder is above it.
    Above,
}

const WRITE_DATA: u32 = 0x2;
const APPEND: u32 = 0x4;
const WRITE_EA: u32 = 0x10;
const DELETE_CHILD: u32 = 0x40;
const WRITE_ATTRIBUTES: u32 = 0x100;
const DELETE: u32 = 0x1_0000;
const WRITE_DAC: u32 = 0x4_0000;
const WRITE_OWNER: u32 = 0x8_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_ALL: u32 = 0x1000_0000;

fn changes(mask: u32, reach: Reach) -> bool {
    let above = DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_WRITE | GENERIC_ALL;
    let folder = above | WRITE_DATA | APPEND | WRITE_EA | WRITE_ATTRIBUTES;
    mask & if reach == Reach::TheFolder { folder } else { above } != 0
}

/// The rights SDDL writes as two letters, and what each is as a number. Any right it has no letters for is written in hex.
fn right_bits(token: &str) -> u32 {
    match token {
        "GA" => GENERIC_ALL,
        "GW" => GENERIC_WRITE,
        "GR" => 0x8000_0000,
        "GX" => 0x2000_0000,
        "FA" => 0x1f_01ff,
        "FR" => 0x12_0089,
        "FW" => 0x12_0116,
        "FX" => 0x12_00a0,
        "SD" => DELETE,
        "RC" => 0x2_0000,
        "WD" => WRITE_DAC,
        "WO" => WRITE_OWNER,
        "CC" => 0x1,
        "DC" => WRITE_DATA,
        "LC" => APPEND,
        "SW" => 0x8,
        "RP" => WRITE_EA,
        "WP" => 0x20,
        "DT" => DELETE_CHILD,
        "LO" => 0x80,
        "CR" => 0x100,
        _ => 0,
    }
}

/// The rights field of one entry: `0x1200a9`, or two-letter rights run together (`FRFX`).
fn rights_mask(rights: &str) -> u32 {
    if let Some(hex) = rights.strip_prefix("0x").or_else(|| rights.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).unwrap_or(u32::MAX);
    }
    rights.as_bytes().chunks(2).map(|c| right_bits(std::str::from_utf8(c).unwrap_or(""))).fold(0, |all, bit| all | bit)
}

/// Who besides the administrators and `allowed` (the service's own account, when it may; an entry ending in `*` is a prefix, so
/// `S-1-5-80-*` is any service's account) can change the thing whose access
/// list is `sddl` (a DACL, or an `O:...D:...` text), each named once, in the order the list gives them. An entry that only
/// applies to what is inside the folder (inherit-only) does not change the folder itself and is not counted. A denial is
/// not counted either: a list is read as its allowances, so a person is told of more than can happen, never of less.
pub fn who_can_change(sddl: &str, allowed: &[&str], reach: Reach) -> Vec<String> {
    let Some(dacl) = sddl.find("D:").map(|at| &sddl[at + 2..]) else { return vec![say_who("WD").name] };
    let dacl = dacl.split("S:").next().unwrap_or("");
    if dacl.contains("NO_ACCESS_CONTROL") {
        return vec![say_who("WD").name];
    }
    let mut out: Vec<String> = Vec::new();
    for entry in dacl.split('(').skip(1) {
        let fields: Vec<&str> = entry.split(')').next().unwrap_or("").split(';').collect();
        let get = |i: usize| fields.get(i).copied().unwrap_or("");
        let (kind, flags, rights, principal) = (get(0), get(1), get(2), get(5));
        let inherit_only = flags.as_bytes().chunks(2).any(|f| f == b"IO");
        if !matches!(kind, "A" | "OA" | "XA" | "ZA") || inherit_only || principal.is_empty() || !changes(rights_mask(rights), reach) {
            continue;
        }
        let known = ADMINS.iter().chain(allowed.iter()).any(|ok| match ok.strip_suffix('*') {
            Some(prefix) => principal.len() >= prefix.len() && principal[..prefix.len()].eq_ignore_ascii_case(prefix),
            None => ok.eq_ignore_ascii_case(principal),
        });
        let who = say_who(&principal.to_ascii_uppercase()).name;
        if !known && !out.contains(&who) {
            out.push(who);
        }
    }
    out
}

/// The owner in an `O:...` text, as the alias or SID it names.
pub fn owner_of(sddl: &str) -> Option<&str> {
    let rest = sddl.strip_prefix("O:")?;
    let end = ["G:", "D:", "S:"].iter().filter_map(|m| rest.find(m)).min().unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Is the owner of the thing with this text an administrator of the computer? A folder owned by anyone else keeps the right
/// to change its own list whatever the list says, so such a folder is not one to put a service's program or data in.
pub fn owned_by_administrators(sddl: &str) -> bool {
    owner_of(sddl).is_some_and(|o| ADMINS.iter().any(|a| a.eq_ignore_ascii_case(o)) && !o.eq_ignore_ascii_case("CO"))
}

/// What is wrong with a folder the service would use, in words, or `None` when it is fit. `existing` is its owner-and-list
/// text when the folder is already there. Used for a folder before it is claimed (an existing one made by someone else could
/// have been made to be taken over) and again after the list was set.
pub fn folder_problem(folder: &Path, owner_and_list: &str, allowed: &[&str], reach: Reach) -> Option<String> {
    if !owned_by_administrators(owner_and_list) {
        let owner = owner_of(owner_and_list).map(|o| say_who(o).name).unwrap_or_else(|| "someone".to_string());
        return Some(format!(
            "{} is owned by {owner}, not by the administrators, so its owner can change who may use it whatever its list says. Remove it (from a PowerShell opened as Administrator: Remove-Item -Recurse \"{}\") and run this again",
            folder.display(),
            folder.display()
        ));
    }
    let who = who_can_change(owner_and_list, allowed, reach);
    if who.is_empty() {
        return None;
    }
    Some(format!(
        "{} can be changed by {}, and AgentMux Headless runs its program and keeps its data there as its own account, so only the administrators may. To fix it, run from a PowerShell opened as Administrator: icacls \"{}\" /inheritance:r /grant:r \"SYSTEM:(OI)(CI)F\" /grant:r \"Administrators:(OI)(CI)F\" and run this again",
        folder.display(),
        who.join(", "),
        folder.display()
    ))
}

/// What a person is told when a folder above the service's can be changed by someone who is not an administrator: who, and the
/// command that takes them out of its list (a name that is a SID is removed by that SID).
pub fn above_problem(above: &Path, who: &[String]) -> String {
    let removals: String = who.iter().filter(|w| w.starts_with("S-1-")).map(|w| format!(" /remove:g \"*{w}\"")).collect();
    format!(
        "{} can be changed by {}, so a person who is not an administrator could replace the folder AgentMux Headless runs from. Take them out of its list, from a PowerShell opened as Administrator{}, and run this again",
        above.display(),
        who.join(", "),
        if removals.is_empty() { String::new() } else { format!(": icacls \"{}\"{removals}", above.display()) }
    )
}

/// One file of a package: where it is under the package's folder (with forward slashes), its size and its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub rel: String,
    pub size: u64,
    pub sha256: String,
}

/// The one digest that says which files a package is made of and what is in each: the SHA-256 of one line per file,
/// `<path, lower-case, forward slashes>\t<size>\t<sha256>\n`, in path order. Windows folds the case of a path, so two copies
/// that differ only in the case of a name are the same package. The copy in `%ProgramFiles%` is hashed afresh, file by
/// file, and must give this same digest: a file changed, added, left out or swapped between the person's package and the
/// copy shows.
pub fn tree_digest(entries: &[Entry]) -> String {
    let mut lines: Vec<String> = entries.iter().map(|e| format!("{}\t{}\t{}\n", e.rel.to_lowercase(), e.size, e.sha256.to_lowercase())).collect();
    lines.sort();
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Is this a digest as [`tree_digest`] gives it: 64 lower-case hex digits?
pub fn digest_is_valid(digest: &str) -> bool {
    digest.len() == 64 && digest.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// The files a package must hold to be AgentMux Headless for Windows (the ones `install.ps1` checks): what is missing, if
/// anything. `rels` are the package's files, lower-cased and with forward slashes.
pub fn missing_from_package(rels: &[String]) -> Vec<&'static str> {
    ["bin/agentmux.exe", "bin/agentmux-srv.exe", "web/index.html"].into_iter().filter(|want| !rels.iter().any(|r| r.eq_ignore_ascii_case(want))).collect()
}

/// Why the digest the administrator step was given cannot be taken, or `None` when it is the digest of the package it read. The step
/// is not a command to type: it is started by `agentmux service install` with the digest of what that command hashed and held, and
/// it installs nothing without one.
pub fn digest_problem(given: Option<&str>, actual: &str) -> Option<String> {
    match given {
        None => Some("this step is started by `agentmux service install`, which gives it the digest of the package it checked: run that instead".to_string()),
        Some(d) if !digest_is_valid(d) => Some("the digest the step was given is not a SHA-256 digest: run `agentmux service install`".to_string()),
        Some(d) if d != actual => Some("the package is not the one that was checked before the administrator prompt (it changed): nothing is installed".to_string()),
        Some(_) => None,
    }
}

/// Why this is not an account that may be given read on the server's key, or `None`. The person is named by their own account's
/// SID (`whoami /user` says it); a group or a well-known SID would let everyone in it read the key.
pub fn reader_problem(sid: &str) -> Option<String> {
    if agentmux_common::owner_only::is_account_sid(sid) {
        return None;
    }
    Some(format!("{sid:?} is not the SID of a person's own account, and a group or a well-known one (Everyone, Users, Authenticated Users, Anonymous) would let all of them read the server's key. Pass the SID `whoami /user` gives for the account that signs in, or leave --reader out"))
}

/// The sentence a refusal for a missing administrator gives, the one a person reads whoever ran the command (C.3).
pub const NEEDS_ADMINISTRATOR: &str = "this needs an administrator: run it from a PowerShell opened as Administrator";

/// What a person is told when the machine belongs to a domain (C.6).
pub const DOMAIN_JOINED: &str = "this computer is joined to a domain. The service's account signs in to the network as this computer, so a program an agent runs in it can reach what the computer can reach. Give --allow-domain-joined to install it anyway";

/// The words printed once the service is installed and running, about what a person does next (B.3, D.1).
pub fn after_install(service: &str, program: &Path, data: &Path) -> Vec<String> {
    vec![
        format!("The service {service} is installed and running. Its program is {}; its data is in {}.", program.display(), data.display()),
        "It starts when this computer starts, before anyone signs in, and keeps running after you sign out.".to_string(),
        "It runs as its own account, not yours, so it starts empty: sign each agent in through the browser. Run `agentmux open` in a normal terminal to open the browser.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVC: &str = "S-1-5-80-111-222-333-444-555";

    #[test]
    fn each_channel_has_its_own_program_folder_and_data_root() {
        let l = layout(Path::new("C:\\Program Files"), Path::new("C:\\ProgramData"), "headless", "0.60.38");
        assert_eq!(l.program_root, PathBuf::from("C:\\Program Files").join("AgentMux").join("Headless"));
        assert_eq!(l.version_dir, l.program_root.join("0.60.38"));
        assert_eq!(l.current, l.program_root.join("current"));
        assert_eq!(l.exe, l.program_root.join("current").join("bin").join("agentmux.exe"));
        assert_eq!(l.data_root, PathBuf::from("C:\\ProgramData").join("AgentMux").join("Headless"));
        let other = layout(Path::new("C:\\Program Files"), Path::new("C:\\ProgramData"), "test", "0.60.38");
        assert_eq!(other.data_root, PathBuf::from("C:\\ProgramData").join("AgentMux").join("Headless-test"));
        assert_ne!(other.program_root, l.program_root);
    }

    #[test]
    fn the_service_is_started_on_the_channel_the_data_root_and_the_port_asked_for() {
        let home = PathBuf::from("C:\\ProgramData\\AgentMux\\Headless");
        assert_eq!(launch_arguments("headless", &home, None), ["--channel", "headless", "--home", "C:\\ProgramData\\AgentMux\\Headless", "service", "run"]);
        assert_eq!(launch_arguments("test", &home, Some(7500))[6..], ["--port", "7500"]);
    }

    #[test]
    fn the_step_s_log_is_named_for_the_channel_and_sits_beside_the_program_folder() {
        assert_eq!(log_name("headless"), "Headless-last-run.log");
        assert_eq!(log_name("test"), "Headless-test-last-run.log");
    }

    #[test]
    fn the_key_and_the_port_are_found_in_the_channel_s_folder_of_the_data_root() {
        let key = key_file(Path::new("C:\\ProgramData\\AgentMux\\Headless"), "headless", "0.60.38");
        assert_eq!(key, PathBuf::from("C:\\ProgramData\\AgentMux\\Headless").join("channels").join("headless").join("versions").join("0.60.38").join("data").join("srv-auth-key"));
        assert_eq!(port_file(&key).file_name().unwrap(), "srv-auth-key.port");
        assert_eq!(port_file(&key).parent(), key.parent());
    }

    #[test]
    fn a_version_is_one_folder_name() {
        for ok in ["0.60.38", "1.2.3-rc.1", "0.60.38+build5"] {
            assert!(version_is_valid(ok), "{ok}");
        }
        for bad in ["", "..", "0..1", "1/2", "1\\2", "a1", "1 2", "1.2:3"] {
            assert!(!version_is_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_program_folder_is_written_by_the_administrators_alone() {
        let sddl = program_sddl(SVC);
        assert_eq!(sddl, format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;{SVC})(A;OICI;0x1200a9;;;BU)"));
        // Read and execute for the service and for the users is not a way to change it: nobody is named.
        assert!(who_can_change(&sddl, &[], Reach::TheFolder).is_empty());
    }

    #[test]
    fn the_data_root_names_the_service_and_the_administrators_and_not_the_person() {
        let sddl = data_sddl(SVC);
        assert_eq!(sddl, format!("D:P(A;OICI;FA;;;{SVC})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"));
        assert!(!sddl.contains("BU") && !sddl.contains("S-1-5-21"));
        assert!(who_can_change(&sddl, &[SVC], Reach::TheFolder).is_empty());
        // The service's own account may write it; the program folder may not give it that.
        assert_eq!(who_can_change(&sddl, &[], Reach::TheFolder), vec![SVC.to_string()]);
    }

    #[test]
    fn a_folder_is_changed_by_whoever_a_list_lets_write_and_a_read_only_entry_is_not() {
        let user = "S-1-5-21-1-2-3-1001";
        let list = format!("D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIID;0x1200a9;;;BU)(A;OICIID;FRFX;;;AU)(A;;FW;;;{user})(A;OICIIO;FA;;;CO)");
        assert_eq!(who_can_change(&list, &[], Reach::TheFolder), vec![user.to_string()]);
        // Every kind of write is one: generic, file, append, attributes, delete, the list, the owner.
        for rights in ["GW", "GA", "FA", "FW", "0x2", "0x4", "0x100", "0x10000", "0x40000", "0x80000", "WD", "WO", "SD"] {
            assert_eq!(who_can_change(&format!("D:(A;;{rights};;;BU)"), &[], Reach::TheFolder), vec!["all Users of this computer"], "{rights}");
        }
        for rights in ["GR", "GX", "FR", "FX", "FRFX", "0x1200a9", "RC"] {
            assert!(who_can_change(&format!("D:(A;;{rights};;;BU)"), &[], Reach::TheFolder).is_empty(), "{rights}");
        }
    }

    #[test]
    fn an_entry_for_what_is_inside_and_a_denial_do_not_change_the_folder() {
        assert!(who_can_change("D:(A;OICIIO;FA;;;BU)", &[], Reach::TheFolder).is_empty(), "inherit-only: applies to what is inside");
        assert!(who_can_change("D:(D;;FA;;;BU)", &[], Reach::TheFolder).is_empty(), "a denial is not an allowance");
        assert_eq!(who_can_change("D:NO_ACCESS_CONTROL", &[], Reach::TheFolder), vec!["Everyone"], "no list at all lets everyone in");
        assert_eq!(who_can_change("D:", &[], Reach::TheFolder), Vec::<String>::new(), "an empty list lets nobody in");
    }

    #[test]
    fn a_folder_above_may_let_users_create_entries_but_not_replace_what_is_below() {
        // What `%ProgramData%` gives the users: create files and folders in it.
        let programdata = "D:PAI(A;OICIIO;FA;;;CO)(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;BU)(A;;0x6;;;BU)";
        assert_eq!(who_can_change(programdata, &[], Reach::TheFolder), vec!["all Users of this computer"]);
        assert!(who_can_change(programdata, &[], Reach::Above).is_empty());
        for rights in ["0x40", "SD", "WD", "WO", "GA", "FA"] {
            assert_eq!(who_can_change(&format!("D:(A;;{rights};;;BU)"), &[], Reach::Above), vec!["all Users of this computer"], "{rights}");
        }
    }

    #[test]
    fn a_folder_owned_by_a_person_is_not_fit_whatever_its_list_says() {
        let list = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
        assert!(owned_by_administrators(&format!("O:BA{list}")));
        assert!(owned_by_administrators(&format!("O:SYG:SY{list}")));
        assert!(owned_by_administrators(&format!("O:S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464G:SY{list}")));
        assert!(!owned_by_administrators(&format!("O:S-1-5-21-1-2-3-1001G:S-1-5-21-1-2-3-513{list}")));
        assert!(!owned_by_administrators(&format!("O:CO{list}")));
        assert!(!owned_by_administrators(list), "no owner is not an administrator");
        assert_eq!(owner_of("O:BAG:SYD:P"), Some("BA"));
        assert_eq!(owner_of("O:S-1-5-21-1-2-3-1001D:P(A;;FA;;;SY)"), Some("S-1-5-21-1-2-3-1001"));
        let said = folder_problem(Path::new("C:\\ProgramData\\AgentMux"), "O:S-1-5-21-1-2-3-1001G:S-1-5-21-1-2-3-513D:P(A;;FA;;;SY)", &[], Reach::TheFolder).unwrap();
        assert!(said.contains("is owned by S-1-5-21-1-2-3-1001") && said.contains("Remove-Item"), "{said}");
    }

    #[test]
    fn a_fit_folder_has_no_problem_and_an_open_one_is_told_with_the_command_that_fixes_it() {
        let fit = format!("O:BAG:SY{}", program_sddl(SVC));
        assert_eq!(folder_problem(Path::new("C:\\Program Files\\AgentMux\\Headless"), &fit, &[], Reach::TheFolder), None);
        let open = "O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;BU)";
        let said = folder_problem(Path::new("C:\\Program Files\\AgentMux\\Headless"), open, &[], Reach::TheFolder).unwrap();
        assert!(said.contains("can be changed by all Users of this computer") && said.contains("icacls \"C:\\Program Files\\AgentMux\\Headless\" /inheritance:r"), "{said}");
    }

    #[test]
    fn a_folder_above_that_a_person_can_change_is_told_with_the_command_that_takes_them_out() {
        let said = above_problem(Path::new("C:\\"), &["S-1-5-21-1-2-3-1003".to_string(), "all Users of this computer".to_string()]);
        assert!(said.contains("C:\\ can be changed by S-1-5-21-1-2-3-1003, all Users of this computer"), "{said}");
        assert!(said.contains("icacls \"C:\\\" /remove:g \"*S-1-5-21-1-2-3-1003\""), "{said}");
        assert!(!above_problem(Path::new("C:\\x"), &["Everyone".to_string()]).contains("icacls"));
    }

    fn entry(rel: &str, size: u64, sha: &str) -> Entry {
        Entry { rel: rel.to_string(), size, sha256: sha.to_string() }
    }

    #[test]
    fn the_digest_of_a_package_does_not_depend_on_the_order_or_the_case_of_a_name_and_nothing_else_matches() {
        let a = vec![entry("bin/agentmux.exe", 10, &"a".repeat(64)), entry("web/index.html", 5, &"b".repeat(64))];
        let b = vec![entry("WEB/Index.html", 5, &"B".repeat(64)), entry("bin/agentmux.exe", 10, &"a".repeat(64))];
        assert_eq!(tree_digest(&a), tree_digest(&b));
        assert!(digest_is_valid(&tree_digest(&a)));
        let mut swapped = a.clone();
        swapped[0].sha256 = "c".repeat(64);
        assert_ne!(tree_digest(&a), tree_digest(&swapped), "a file swapped");
        let mut longer = a.clone();
        longer[1].size = 6;
        assert_ne!(tree_digest(&a), tree_digest(&longer), "a file of another size");
        let mut added = a.clone();
        added.push(entry("bin/extra.dll", 1, &"d".repeat(64)));
        assert_ne!(tree_digest(&a), tree_digest(&added), "a file added");
        assert_ne!(tree_digest(&a), tree_digest(&a[..1]), "a file left out");
        let moved = vec![entry("bin/agentmux.exe", 10, &"a".repeat(64)), entry("web/other.html", 5, &"b".repeat(64))];
        assert_ne!(tree_digest(&a), tree_digest(&moved), "a file under another name");
    }

    #[test]
    fn only_a_lower_case_sha256_is_a_digest() {
        assert!(digest_is_valid(&"0123456789abcdef".repeat(4)));
        for bad in ["", "abc", &"A".repeat(64), &"g".repeat(64), &"a".repeat(65)] {
            assert!(!digest_is_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_step_installs_nothing_without_the_digest_of_the_package_that_was_checked() {
        let digest = "ab".repeat(32);
        assert_eq!(digest_problem(Some(&digest), &digest), None);
        assert!(digest_problem(None, &digest).unwrap().contains("agentmux service install"), "no digest given");
        assert!(digest_problem(Some("abc"), &digest).unwrap().contains("not a SHA-256 digest"), "not a digest");
        assert!(digest_problem(Some(&"AB".repeat(32)), &digest).is_some(), "not lower case");
        assert!(digest_problem(Some(&"cd".repeat(32)), &digest).unwrap().contains("it changed"), "another package");
    }

    #[test]
    fn a_reader_is_refused_unless_it_is_a_persons_own_account_and_the_refusal_says_what_to_pass() {
        assert_eq!(reader_problem("S-1-5-21-1-2-3-1001"), None);
        for bad in ["S-1-1-0", "S-1-5-11", "S-1-5-32-545", "S-1-5-7", "S-1-5-18", "S-1-5-21-1-2-3-513", "S-1-5-21-1-2-3-501", "Everyone"] {
            let said = reader_problem(bad).unwrap();
            assert!(said.contains("whoami /user") && said.contains(&format!("{bad:?}")), "{bad}: {said}");
        }
    }

    #[test]
    fn a_service_account_may_be_allowed_by_prefix_and_nobody_else_with_that_prefix_is() {
        let sddl = "D:P(A;OICI;FA;;;S-1-5-80-1-2-3-4-5)(A;OICI;FA;;;S-1-5-21-1-2-3-1001)";
        assert_eq!(who_can_change(sddl, &["S-1-5-80-*"], Reach::TheFolder), vec!["S-1-5-21-1-2-3-1001"]);
        assert_eq!(who_can_change(sddl, &[], Reach::TheFolder).len(), 2);
    }

    /// The test switch for one check exists only in a debug build: the function that reads it is compiled for debug assertions
    /// alone, so the name is not in a release build, and no release profile or workflow turns debug assertions on.
    #[test]
    fn the_test_switch_is_compiled_only_into_debug_builds_and_no_release_build_turns_debug_assertions_on() {
        let source = include_str!("winsvc_apply.rs");
        let lines: Vec<&str> = source.lines().collect();
        let uses: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains("AGENTMUX_TEST_SKIP_ABOVE_CHECK")).map(|(i, _)| i).collect();
        assert_eq!(uses.len(), 1, "the name is used in one place");
        assert_eq!(lines[uses[0] - 2].trim(), "#[cfg(debug_assertions)]", "and only where debug assertions are on");
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let mut texts = vec![std::fs::read_to_string(workspace.join("Cargo.toml")).unwrap_or_default()];
        texts.extend(["config.toml", "config"].iter().filter_map(|f| std::fs::read_to_string(workspace.join(".cargo").join(f)).ok()));
        if let Ok(dir) = std::fs::read_dir(workspace.join(".github").join("workflows")) {
            texts.extend(dir.flatten().filter_map(|f| std::fs::read_to_string(f.path()).ok()));
        }
        for text in texts {
            for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
                let squeezed: String = line.split_whitespace().collect();
                assert!(!squeezed.contains("debug-assertions=true") && !squeezed.contains("Cdebug-assertions"), "a build turns debug assertions on: {line}");
            }
        }
    }

    #[test]
    fn a_package_must_hold_the_programs_and_the_web_page() {
        let all: Vec<String> = ["bin/agentmux.exe", "bin/agentmux-srv.exe", "web/index.html", "version"].iter().map(|s| s.to_string()).collect();
        assert!(missing_from_package(&all).is_empty());
        assert_eq!(missing_from_package(&all[1..]), vec!["bin/agentmux.exe"]);
        assert_eq!(missing_from_package(&[]), vec!["bin/agentmux.exe", "bin/agentmux-srv.exe", "web/index.html"]);
        let cased: Vec<String> = ["BIN/AgentMux.exe", "bin/agentmux-srv.exe", "Web/index.html"].iter().map(|s| s.to_string()).collect();
        assert!(missing_from_package(&cased).is_empty());
    }

    #[test]
    fn what_a_person_is_told_after_the_install_names_the_program_the_data_and_the_browser() {
        let lines = after_install("AgentMuxHeadless", Path::new("C:\\p\\agentmux.exe"), Path::new("C:\\d"));
        let text = lines.join("\n");
        assert!(text.contains("AgentMuxHeadless") && text.contains("C:\\p\\agentmux.exe") && text.contains("C:\\d"));
        assert!(text.contains("before anyone signs in") && text.contains("sign each agent in through the browser") && text.contains("agentmux open"));
    }
}
