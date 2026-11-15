//! Live check of the Personal Memory mount against a real Docker daemon:
//!   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_personal_memory
//!
//! A note written natively is read in the container, and one written in the container is on
//! the host; and the agent can still write its session files beside the mount.

use futures_util::StreamExt as _;

use super::*;

async fn run(cm: &ContainerManager, name: &str, script: &str) -> String {
    let session = cm.exec(name, &["sh".into(), "-c".into(), script.into()], None, &[], false).await.expect("exec");
    drop(session.input);
    let mut out = String::new();
    let mut stream = session.output;
    while let Some(Ok(chunk)) = stream.next().await {
        out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
    }
    out
}

#[tokio::test]
#[ignore]
async fn itest_personal_memory_is_the_same_files_natively_and_in_the_container() {
    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");
    let root = std::env::temp_dir().join(format!("agentmux-p5-itest-{}", std::process::id()));
    let config = root.join("claude");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let host_memory = personal_memory_host_dir(Some(&config.to_string_lossy()), &work.to_string_lossy()).unwrap();
    std::fs::write(std::path::Path::new(&host_memory).join("MEMORY.md"), "- [native note](native.md)\n").unwrap();
    std::fs::write(std::path::Path::new(&host_memory).join("native.md"), "written on the host\n").unwrap();

    let name = "agentmux-itest-p5";
    let image = "ghcr.io/agentmuxai/agent-base:0.59.41";
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
    let spec = ContainerMountSpec { memory_host_dir: Some(host_memory.clone()), ..Default::default() };
    cm.ensure_running_observed(name, image, &[], &[], &spec, &|_| {}).await.expect("ensure_running");

    let dir = container_memory_dir();
    let seen = run(&cm, name, &format!("cat {dir}/MEMORY.md {dir}/native.md")).await;
    let wrote = run(&cm, name, &format!("printf 'written in the container\\n' > {dir}/from-container.md && echo ok")).await;
    let session = run(&cm, name, &format!("touch {dir}/../session.jsonl && echo ok")).await;
    let only = run(&cm, name, "ls -A /home/agent/.claude/projects; echo --; ls -A /home/agent/.claude/projects/-workspace; echo --; ls -A /home/agent/.claude").await;
    println!("{seen}\n{wrote}\n{session}\n{only}");
    let on_host = std::fs::read_to_string(std::path::Path::new(&host_memory).join("from-container.md")).unwrap_or_default();

    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
    let _ = std::fs::remove_dir_all(&root);

    assert!(seen.contains("native note") && seen.contains("written on the host"), "the native note is read in the container: {seen}");
    assert!(wrote.contains("ok") && on_host == "written in the container\n", "the container's note is on the host: {wrote} / {on_host:?}");
    assert!(session.contains("ok"), "the agent can write session files beside the memory folder: {session}");
    assert_eq!(
        only.trim(),
        "-workspace\n--\nmemory\nsession.jsonl\n--\nprojects",
        "the volume holds only the memory folder and what the agent wrote itself: {only}"
    );
}

/// The bound credentials file and `~/.claude` are the agent's alone once the container is up:
///   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_credentials_are_the_agents_alone
#[cfg(unix)]
#[tokio::test]
#[ignore]
async fn itest_credentials_are_the_agents_alone() {
    use std::os::unix::fs::PermissionsExt;
    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");
    let root = std::env::temp_dir().join(format!("agentmux-cred-itest-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let creds = root.join(".credentials.json");
    std::fs::write(&creds, "{}").unwrap();
    std::fs::set_permissions(&creds, std::fs::Permissions::from_mode(0o666)).unwrap();

    let name = "agentmux-itest-cred";
    let image = "ghcr.io/agentmuxai/agent-base:0.60.3";
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
    let spec = ContainerMountSpec { claude_config_host_dir: Some(root.to_string_lossy().into_owned()), ..Default::default() };
    cm.ensure_running_observed(name, image, &[], &[], &spec, &|_| {}).await.expect("ensure_running");
    let seen = run(&cm, name, "stat -c '%u:%g %a' /home/agent/.claude /home/agent/.claude/.credentials.json; id -u").await;
    let on_host = std::fs::metadata(&creds).unwrap().permissions().mode() & 0o777;

    let _ = cm.remove(name, true).await;
    for volume in [format!("agentmux-claude-{name}"), format!("agentmux-cli-{name}")] {
        let _ = cm.inner.docker.remove_volume(&volume, None).await;
    }
    let _ = std::fs::remove_dir_all(&root);

    let lines: Vec<&str> = seen.lines().map(str::trim).collect();
    let me = lines.last().copied().unwrap_or("");
    assert_eq!(lines[0], format!("{me}:{me} 700"), "~/.claude: {seen}");
    assert_eq!(lines[1], format!("{me}:{me} 600"), "the credentials file: {seen}");
    assert_eq!(on_host, 0o600, "the host file was 0666");
}
