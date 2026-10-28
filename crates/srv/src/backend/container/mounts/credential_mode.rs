//! Who owns, and who can read, the credentials inside a container agent.
//!
//! The bound account's `.credentials.json` is a single-file bind of a host file, so what a
//! container sees of it is whatever the host presents: a Linux host shows its own owner and
//! mode, Docker Desktop shows a synthesized root-owned `rwxrwxrwx`. The agent's `~/.claude`
//! volume is created from the image. When a container starts (or has been restarted since it was
//! last looked at) a root exec tightens what is the turn user's: `~/.claude` and its top-level
//! files to the agent alone, and the credentials file to `600`. A credentials file that root owns is
//! handed to the turn user first. One owned by a real host user is left exactly as it is: through the
//! bind a `chown` would change the host's file, and a `chmod 600` could lock the agent out of a file
//! it can read today. It then reads the file back, and says in the log what it is.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use futures_util::StreamExt as _;

use super::{ContainerManager, ContainerMountSpec, CLAUDE_CREDENTIALS_FILE, CONTAINER_CLAUDE_DIR};

/// The script. `$1` the config dir, `$2` the credentials file, `$3`:`$4` the turn user. Prints the
/// credentials file's `uid:gid mode` last, or `none` when there is no file. Nothing is changed that
/// the turn user does not own (or that root owned and was just handed over): taking a mode from
/// someone else's file could lock the agent out of it.
const SCRIPT: &str = r#"d="$1"; f="$2"
if [ -d "$d" ] && [ "$(stat -c %u "$d")" = "$3" ]; then
  chmod 700 "$d"
  find "$d" -maxdepth 1 -type f ! -path "$f" -user "$3" -exec chmod go-rwx {} +
fi
if [ -f "$f" ] && [ ! -L "$f" ]; then
  [ "$(stat -c %u "$f")" = 0 ] && chown "$3:$4" "$f"
  [ "$(stat -c %u "$f")" = "$3" ] && chmod 600 "$f"
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
    /// It is this `uid:gid mode` and not the agent's: a real host user's file, or a layer that ignores the change.
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

/// The container's `StartedAt` when its credentials were last looked at, so a restart or a
/// recreate is looked at again and nothing else is.
fn secured() -> &'static Mutex<HashMap<String, String>> {
    static SECURED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    SECURED.get_or_init(Default::default)
}

impl ContainerManager {
    /// Everything a new container's home needs before its first turn.
    pub(in crate::backend::container) async fn prepare_home(&self, container_name: &str, spec: &ContainerMountSpec) {
        self.own_memory_parents(container_name, spec).await;
        self.secure_credentials_once(container_name).await;
    }

    /// Once per start of the container: a running one that was not looked at, a restarted one, a new one.
    pub(in crate::backend::container) async fn secure_credentials_once(&self, container_name: &str) {
        let started = self.inner.docker.inspect_container(container_name, None).await.ok().and_then(|c| c.state?.started_at).unwrap_or_default();
        if !started.is_empty() && secured().lock().unwrap().get(container_name) == Some(&started) {
            return;
        }
        if self.secure_credentials(container_name).await && !started.is_empty() {
            secured().lock().unwrap().insert(container_name.to_string(), started);
        }
    }

    /// Best-effort: a failure is logged and the container carries on.
    async fn secure_credentials(&self, container_name: &str) -> bool {
        let Some((uid, gid)) = self.exec_identity(container_name).await else {
            tracing::warn!(container = container_name, "container agent: can't tell which user runs turns; the credentials stay as the host presents them");
            return false;
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
                match outcome(&out, uid, gid) {
                    Some(Outcome::Private) => tracing::info!(container = container_name, "container agent: credentials are the agent's alone (600)"),
                    Some(Outcome::NoFile) => {}
                    Some(Outcome::Presented(seen)) => tracing::warn!(
                        container = container_name,
                        seen,
                        "container agent: the credentials file is {seen} (uid:gid mode) as the container sees it, not the agent's own, so it was left as it is; \
                         if the agent can't sign in, the host file's owner is the thing to check"
                    ),
                    None => tracing::warn!(container = container_name, "container agent: couldn't read back the credentials file's owner"),
                }
                true
            }
            Ok((code, _)) => {
                tracing::warn!(container = container_name, ?code, "container agent: tightening the credentials failed");
                false
            }
            Err(e) => {
                tracing::warn!(container = container_name, error = %e, "container agent: couldn't tighten the credentials");
                false
            }
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
            let run_as = |uid: u32| {
                let out = Command::new("sh")
                    .args(["-c", SCRIPT, "sh"])
                    .arg(&dir)
                    .arg(&creds)
                    .arg(uid.to_string())
                    .arg(me.gid().to_string())
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
                String::from_utf8_lossy(&out.stdout).into_owned()
            };

            let run = || run_as(me.uid());

            // Files that belong to somebody else (here: the turn user is another uid) are left exactly
            // as they are, so a `600` can't lock the agent out of a file it reads today.
            let out = run_as(me.uid() + 1);
            assert_eq!(super::super::outcome(&out, me.uid() as u64 + 1, me.gid() as u64), Some(super::super::Outcome::Presented(format!("{}:{} 666", me.uid(), me.gid()))), "{out}");
            assert_eq!((mode(&dir), mode(&creds), mode(&settings)), (0o755, 0o666, 0o666));

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
