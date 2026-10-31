use super::*;

/// A fake global npm install: `<root>/npm/node_modules/<package>` with the script, a README and a dependency.
fn install(root: &Path, package: &str, version: &str, script: &str) -> PathBuf {
    let dir = root.join("npm").join("node_modules").join(package);
    std::fs::create_dir_all(dir.join("node_modules/dep")).unwrap();
    std::fs::create_dir_all(dir.join(Path::new(script).parent().unwrap())).unwrap();
    std::fs::write(dir.join("package.json"), format!("{{\"name\":\"{package}\",\"version\":\"{version}\"}}")).unwrap();
    std::fs::write(dir.join(script), "// cli").unwrap();
    std::fs::write(dir.join("node_modules/dep/index.js"), "// dep").unwrap();
    std::fs::write(dir.join("README.md"), "internal notes").unwrap();
    dir
}

fn which_in(root: &Path) -> impl Fn(&str) -> Option<PathBuf> + '_ {
    move |c| Some(root.join("npm").join(format!("{c}.cmd")))
}

#[test]
fn a_windows_style_install_is_found_beside_its_command() {
    let root = tempfile::tempdir().unwrap();
    let dir = install(root.path(), "@a5af/secrets", "1.10.0", "bin/secrets.js");
    let found = find("secrets", &which_in(root.path())).unwrap();
    assert_eq!((found.command, found.version.as_str(), found.dir), ("secrets", "1.10.0", dir));
}

#[cfg(unix)]
#[test]
fn a_unix_style_install_is_found_through_the_link_the_command_is() {
    let root = tempfile::tempdir().unwrap();
    let dir = install(root.path(), "@a5af/deploy-cli", "0.10.2", "dist/cli.js");
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(dir.join("dist/cli.js"), bin.join("deploy")).unwrap();
    let found = find("deploy", &|c| Some(bin.join(c))).unwrap();
    assert_eq!(found.dir.canonicalize().unwrap(), dir.canonicalize().unwrap());
}

#[test]
fn only_the_named_package_with_a_plain_version_and_its_script_counts() {
    let root = tempfile::tempdir().unwrap();
    // Another package where `secrets` should be.
    let dir = install(root.path(), "@a5af/secrets", "1.10.0", "bin/secrets.js");
    std::fs::write(dir.join("package.json"), r#"{"name":"@a5af/other","version":"1.0.0"}"#).unwrap();
    assert_eq!(find("secrets", &which_in(root.path())), None);
    // A version that isn't a plain word never becomes a folder name or a shell word.
    install(root.path(), "@a5af/deploy-cli", "1.0.0; rm -rf /", "dist/cli.js");
    assert_eq!(find("deploy", &which_in(root.path())), None);
    // Not installed, and not a tool of ours.
    assert_eq!(find("secrets", &|_| None), None);
    assert_eq!(find("aws", &which_in(root.path())), None);
}

#[test]
fn the_folder_holds_the_runtime_files_a_launcher_per_tool_and_nothing_internal() {
    let root = tempfile::tempdir().unwrap();
    install(root.path(), "@a5af/secrets", "1.10.0", "bin/secrets.js");
    install(root.path(), "@a5af/deploy-cli", "0.10.2", "dist/cli.js");
    let secrets = find("secrets", &which_in(root.path())).unwrap();
    let deploy = find("deploy", &which_in(root.path())).unwrap();
    let out = tempfile::tempdir().unwrap();
    let dir = render_into(out.path(), &[secrets, deploy]).unwrap();
    assert_eq!(dir, out.path().join("container-tools"));
    let s = dir.join("pkg/secrets-1.10.0");
    assert!(s.join("bin/secrets.js").is_file() && s.join("package.json").is_file() && s.join("node_modules/dep/index.js").is_file());
    assert!(!s.join("README.md").exists(), "docs stay on the host");
    let launcher = std::fs::read_to_string(dir.join("bin/secrets")).unwrap();
    assert!(launcher.contains("exec node /opt/agentmux/tools/pkg/secrets-1.10.0/bin/secrets.js \"$@\""), "{launcher}");
    let launcher = std::fs::read_to_string(dir.join("bin/deploy")).unwrap();
    assert!(launcher.contains("exec node /opt/agentmux/tools/pkg/deploy-0.10.2/dist/cli.js \"$@\""), "{launcher}");
}

#[test]
fn a_tool_the_host_lacks_says_so_in_one_sentence_and_the_other_still_runs() {
    let root = tempfile::tempdir().unwrap();
    install(root.path(), "@a5af/secrets", "1.10.0", "bin/secrets.js");
    let secrets = find("secrets", &which_in(root.path())).unwrap();
    let out = tempfile::tempdir().unwrap();
    let dir = render_into(out.path(), &[secrets]).unwrap();
    let stub = std::fs::read_to_string(dir.join("bin/deploy")).unwrap();
    assert!(stub.contains("this container has the AWS CLI only") && stub.contains("npm i -g @a5af/deploy-cli") && stub.contains("exit 127"), "{stub}");
    assert!(std::fs::read_to_string(dir.join("bin/secrets")).unwrap().contains("exec node"));
    // With nothing installed both are stubs, and no package folder is made.
    let empty = tempfile::tempdir().unwrap();
    let none = render_into(empty.path(), &[]).unwrap();
    assert!(std::fs::read_to_string(none.join("bin/secrets")).unwrap().contains("AWS CLI only"));
    assert_eq!(std::fs::read_dir(none.join("pkg")).unwrap().count(), 0);
}

#[test]
fn an_unchanged_host_is_not_rewritten_and_a_new_version_gets_its_own_folder() {
    let root = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let at = |version: &str| {
        let dir = install(root.path(), "@a5af/secrets", version, "bin/secrets.js");
        find("secrets", &which_in(root.path())).map(|f| (f, dir)).unwrap().0
    };
    let first = at("1.10.0");
    let dir = render_into(out.path(), &[first.clone()]).unwrap();
    let launcher = dir.join("bin/secrets");
    let stamp = std::fs::metadata(&launcher).unwrap().modified().unwrap();
    render_into(out.path(), &[first]).unwrap();
    assert_eq!(std::fs::metadata(&launcher).unwrap().modified().unwrap(), stamp, "nothing changed");
    render_into(out.path(), &[at("1.11.0")]).unwrap();
    assert!(dir.join("pkg/secrets-1.11.0/package.json").is_file());
    assert!(dir.join("pkg/secrets-1.10.0/package.json").is_file(), "a command still running from the old one keeps its files");
    assert!(std::fs::read_to_string(&launcher).unwrap().contains("secrets-1.11.0"));
    let leftovers: Vec<_> = std::fs::read_dir(dir.join("pkg")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect();
    assert!(leftovers.is_empty(), "no staging folder is left: {leftovers:?}");
}

#[cfg(unix)]
#[test]
fn launchers_are_executable_and_a_link_out_of_a_package_is_not_followed() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = install(root.path(), "@a5af/secrets", "1.10.0", "bin/secrets.js");
    let outside = root.path().join("outside.txt");
    std::fs::write(&outside, "private").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("dist-link")).unwrap();
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("dist/leak")).unwrap();
    let found = find("secrets", &which_in(root.path())).unwrap();
    let out = tempfile::tempdir().unwrap();
    let tools = render_into(out.path(), &[found]).unwrap();
    assert_eq!(std::fs::metadata(tools.join("bin/secrets")).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(!tools.join("pkg/secrets-1.10.0/dist/leak").exists());
}

#[test]
fn under_test_nothing_is_asked_of_the_host() {
    assert_eq!(config_dir_for("jcont"), None);
}

/// Run by hand on a host that has the CLIs installed: `cargo test -p agentmux-srv -- --ignored the_hosts_own_install`.
#[test]
#[ignore = "needs secrets and deploy installed on this computer"]
fn the_hosts_own_install_is_found_and_copied() {
    let which = |c: &str| which::which(c).ok();
    let found: Vec<Found> = TOOLS.iter().filter_map(|t| find(t.command, &which)).collect();
    eprintln!("{found:#?}");
    assert_eq!(found.len(), TOOLS.len(), "both CLIs are installed here");
    let out = tempfile::tempdir().unwrap();
    let dir = render_into(out.path(), &found).unwrap();
    for f in &found {
        assert!(dir.join("pkg").join(format!("{}-{}", f.command, f.version)).join("package.json").is_file());
    }
}
