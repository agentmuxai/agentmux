//! Who owns, and who can read, the credentials inside a container agent.
//!
//! The bound account's `.credentials.json` is a single-file bind of a host file, so what a
//! container sees of it is whatever the host presents: a Linux host shows its own owner and
//! mode, Docker Desktop shows a synthesized root-owned `rwxrwxrwx`. The agent's `~/.claude`
//! volume is created from the image. At start (and once per container for ones that were
//! already running) a root exec tightens what it can: `~/.claude` and its top-level files to
//! the agent alone, and the credentials file to `600`, owned by the turn user. It hands a
//! file to the turn user only when root owns it: a file owned by a real host user is that
//! user's, and through the bind a `chown` would change the host's file. It then reads the
//! file back, and says in the log if the host still presents it as something else.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use futures_util::StreamExt as _;

use super::{ContainerManager, ContainerMountSpec, CLAUDE_CREDENTIALS_FILE, CONTAINER_CLAUDE_DIR};

/// The script. `$1` the config dir, `$2` the credentials file, `$3`:`$4` the turn user. Prints the
/// credentials file's `uid:gid mode` last, or `none` when there is no file.
const SCRIPT: &str = r#"d="$1"; f="$2"
[ -d "$d" ] && chmod 700 "$d"
[ -d "$d" ] && find "$d" -maxdepth 1 -type f -exec chmod go-rwx {} +
if [ -f "$f" ] && [ ! -L "$f" ]; then
  [ "$(stat -c %u "$f")" = 0 ] && chown "$3:$4" "$f"
  chmod 600 "$f"
  stat -c '%u:%g %a' "$f"
else
  echo none
fi"#;

/// What the credentials file ended up as, from the script's last line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Owned by the turn user, `600`.
    Private,
    /// There is no credentials file (no account bound, not signed in yet).
    NoFile,
    /// The host presents it as this `uid:gid mode`, whatever was asked.
    Presented(String),
}

pub(super) fn outcome(output: &str, uid: u64, gid: u64) -> Option<Outcome> {
    let last = output.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    if last == "none" {
        return Some(Outcome::NoFile);
    }
    let (owner, mode) = last.split_once(' ')?;
    Some(if owner == format!("{uid}:{gid}") && mode == "600" { Outcome::Private } else { Outcome::Presented(last.to_string()) })
}

fn secured() -> &'static Mutex<HashSet<String>> {
    static SECURED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SECURED.get_or_init(Default::default)
}

impl ContainerManager {
    /// Everything a new container's home needs before its first turn.
    pub(in crate::backend::container) async fn prepare_home(&self, container_name: &str, spec: &ContainerMountSpec) {
        self.own_memory_parents(container_name, spec).await;
        self.secure_credentials(container_name).await;
    }

    /// For a container that is already running: once per container per srv lifetime.
    pub(in crate::backend::container) async fn secure_credentials_once(&self, container_name: &str) {
        if secured().lock().unwrap().contains(container_name) {
            return;
        }
        self.secure_credentials(container_name).await;
    }

    /// Best-effort: a failure is logged and the container carries on.
    pub(super) async fn secure_credentials(&self, container_name: &str) {
        let Some((uid, gid)) = self.exec_identity(container_name).await else {
            tracing::warn!(container = container_name, "container agent: can't tell which user runs turns; the credentials stay as the host presents them");
            return;
        };
        let file = format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}");
        let (uid_arg, gid_arg) = (uid.to_string(), gid.to_string());
        let argv = ["sh", "-c", SCRIPT, "sh", CONTAINER_CLAUDE_DIR, file.as_str(), uid_arg.as_str(), gid_arg.as_str()];
        let ran = async {
            let exec = self
                .inner
                .docker
                .create_exec(
                    container_name,
                    CreateExecOptions { user: Some("root"), cmd: Some(argv.to_vec()), attach_stdout: Some(true), attach_stderr: Some(true), ..Default::default() },
                )
                .await?;
            let mut out = String::new();
            if let StartExecResults::Attached { mut output, .. } =
                self.inner.docker.start_exec(&exec.id, Some(StartExecOptions { detach: false, ..Default::default() })).await?
            {
                while let Some(Ok(chunk)) = output.next().await {
                    out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                }
            }
            Ok::<_, crate::backend::container::ContainerError>((self.inspect_exec(&exec.id).await?, out))
        }
        .await;
        match ran {
            Ok((Some(0), out)) => {
                secured().lock().unwrap().insert(container_name.to_string());
                match outcome(&out, uid, gid) {
                    Some(Outcome::Private) => tracing::info!(container = container_name, "container agent: credentials are the agent's alone (600)"),
                    Some(Outcome::NoFile) => {}
                    Some(Outcome::Presented(seen)) => tracing::warn!(
                        container = container_name,
                        seen,
                        "container agent: the host presents the credentials file as {seen}, not the agent's alone, and the container can't change it; \
                         only this container's processes can reach it, but check who else can read the host file"
                    ),
                    None => tracing::warn!(container = container_name, "container agent: couldn't read back the credentials file's owner"),
                }
            }
            Ok((code, _)) => tracing::warn!(container = container_name, ?code, "container agent: tightening the credentials failed"),
            Err(e) => tracing::warn!(container = container_name, error = %e, "container agent: couldn't tighten the credentials"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_last_line_says_what_the_host_presents() {
        assert_eq!(outcome("1000:1000 600\n", 1000, 1000), Some(Outcome::Private));
        assert_eq!(outcome("none\n", 1000, 1000), Some(Outcome::NoFile));
        assert_eq!(outcome("0:0 777\n", 1000, 1000), Some(Outcome::Presented("0:0 777".into())));
        assert_eq!(outcome("1000:1000 666", 1000, 1000), Some(Outcome::Presented("1000:1000 666".into())));
        assert_eq!(outcome("chmod: nope\n1001:1001 600\n", 1000, 1000), Some(Outcome::Presented("1001:1001 600".into())));
        assert_eq!(outcome("", 1000, 1000), None);
        assert_eq!(outcome("garbage", 1000, 1000), None);
    }

    #[cfg(target_os = "linux")]
    mod on_linux {
        use super::super::SCRIPT;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::process::Command;

        fn mode(p: &std::path::Path) -> u32 {
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777
        }
        fn set(p: &std::path::Path, m: u32) {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        }

        /// A `.claude` dir as an image-made volume leaves it (755, files 644) with a world-writable
        /// credentials file owned by whoever runs the test, the script run as that user.
        #[test]
        fn the_script_makes_the_config_dir_and_credentials_the_agents_alone() {
            let root = std::env::temp_dir().join(format!("agentmux-cred-mode-{}", std::process::id()));
            let dir = root.join(".claude");
            let memory = dir.join("projects/-workspace/memory");
            std::fs::create_dir_all(&memory).unwrap();
            let creds = dir.join(".credentials.json");
            let settings = dir.join("settings.json");
            let note = memory.join("a.md");
            for f in [&creds, &settings, &note] {
                std::fs::write(f, "x").unwrap();
                set(f, 0o666);
            }
            set(&dir, 0o755);
            let me = std::fs::metadata(&dir).unwrap();
            let run = || {
                let out = Command::new("sh")
                    .args(["-c", SCRIPT, "sh"])
                    .arg(&dir)
                    .arg(&creds)
                    .arg(me.uid().to_string())
                    .arg(me.gid().to_string())
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
                String::from_utf8_lossy(&out.stdout).into_owned()
            };

            let out = run();
            assert_eq!(super::super::outcome(&out, me.uid() as u64, me.gid() as u64), Some(super::super::Outcome::Private), "{out}");
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&creds), 0o600);
            assert_eq!(mode(&settings), 0o600, "a top-level file beside it");
            assert_eq!(mode(&note), 0o666, "the memory folder is bound from the host and is not touched");
            run();
            assert_eq!(mode(&creds), 0o600, "running it again changes nothing");

            // A symlink in the credentials' place is not followed, and there being no file is fine.
            std::fs::remove_file(&creds).unwrap();
            assert_eq!(super::super::outcome(&run(), 0, 0), Some(super::super::Outcome::NoFile));
            std::os::unix::fs::symlink(&settings, &creds).unwrap();
            assert_eq!(super::super::outcome(&run(), 0, 0), Some(super::super::Outcome::NoFile));
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
