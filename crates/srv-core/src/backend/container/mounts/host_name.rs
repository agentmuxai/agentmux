//! The machine's own name inside a container agent's container, so `hostname` there (what a PR title's
//! `<name>@<host>` uses; the commit trailers already carry it) says which machine the agent is on, not the
//! container's random id.

/// A host name Docker accepts as a container's hostname: its first label, letters, digits and dashes, at most 63
/// characters. `None` for a name that has none of those.
pub(super) fn container_hostname(raw: Option<&str>) -> Option<String> {
    let first = raw?.trim().split('.').next()?;
    let name: String = first.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(63).collect();
    let name = name.trim_matches('-').to_string();
    (!name.is_empty()).then_some(name)
}

/// The hostname a new container is given: this machine's name as the fleet writes it (`narko`, lower case), the same
/// one the commit trailers carry (`AGENTMUX_GIT_HOST`).
pub(in crate::backend::container) fn host_name_for_new_container() -> Option<String> {
    container_hostname(crate::identity::github_tokens::host_label().as_deref())
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
}
