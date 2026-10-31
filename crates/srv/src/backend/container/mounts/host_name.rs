//! The machine's own name inside a container agent's container, so `hostname` there (what a PR title's
//! `<name>@<host>` and a commit use) says which machine the agent is on, not the container's random id.

/// A host name Docker accepts as a container's hostname: its first label, letters, digits and dashes, at most 63
/// characters. `None` for a name that has none of those.
pub(super) fn container_hostname(raw: Option<&str>) -> Option<String> {
    let first = raw?.trim().split('.').next()?;
    let name: String = first.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(63).collect();
    let name = name.trim_matches('-').to_string();
    (!name.is_empty()).then_some(name)
}

/// The hostname a new container is given, and `AGENTMUX_HOST_NAME` with it in `env` (unless the caller set one).
pub(in crate::backend::container) fn host_name_for_new_container(env: &mut Vec<String>) -> Option<String> {
    let name = container_hostname(whoami::fallible::hostname().ok().as_deref())?;
    if !env.iter().any(|e| e.starts_with("AGENTMUX_HOST_NAME=")) {
        env.push(format!("AGENTMUX_HOST_NAME={name}"));
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_name_becomes_a_hostname_docker_accepts() {
        assert_eq!(container_hostname(Some("charlie")).as_deref(), Some("charlie"));
        assert_eq!(container_hostname(Some("Area54.local")).as_deref(), Some("Area54"), "the first label");
        assert_eq!(container_hostname(Some("  Yas's MacBook_Pro ")).as_deref(), Some("YassMacBookPro"), "odd characters dropped");
        assert_eq!(container_hostname(Some(&"a".repeat(80))).map(|n| n.len()), Some(63));
        assert_eq!(container_hostname(Some("-x-")).as_deref(), Some("x"));
        assert_eq!(container_hostname(Some("___")), None);
        assert_eq!(container_hostname(Some("")), None);
        assert_eq!(container_hostname(None), None);
    }

    #[test]
    fn the_name_is_added_to_the_environment_unless_it_is_already_there() {
        let mut env = vec!["A=1".to_string()];
        let name = host_name_for_new_container(&mut env);
        assert_eq!(name.is_some(), env.iter().any(|e| e.starts_with("AGENTMUX_HOST_NAME=")));
        let mut set = vec!["AGENTMUX_HOST_NAME=mine".to_string()];
        host_name_for_new_container(&mut set);
        assert_eq!(set, ["AGENTMUX_HOST_NAME=mine"]);
    }
}
