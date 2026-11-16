use super::*;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn a_quoted_skill_md_splits_into_name_description_and_body() {
    let (n, d, b) = parse_skill_md("---\nname: \"deploy-check\"\ndescription: 'Use when shipping'\nlicense: MIT\n---\n\nStep 1\n", "fallback");
    assert_eq!((n.as_str(), d.as_str(), b.as_str()), ("deploy-check", "Use when shipping", "Step 1\n"));
}

#[test]
fn a_folded_description_is_joined_and_crlf_is_accepted() {
    let (n, d, b) = parse_skill_md("---\r\nname: pdf\r\ndescription: >\r\n  Read a PDF.\r\n  Use for forms.\r\n---\r\nBody\r\n", "x");
    assert_eq!((n.as_str(), d.as_str(), b.as_str()), ("pdf", "Read a PDF. Use for forms.", "Body\n"));
}

#[test]
fn no_frontmatter_makes_the_whole_text_the_body_under_the_fallback_name() {
    let (n, d, b) = parse_skill_md("# Just notes\n", "notes");
    assert_eq!((n.as_str(), d.as_str(), b.as_str()), ("notes", "", "# Just notes\n"));
    // An opening --- with no closing one is not frontmatter either.
    let (n, _, b) = parse_skill_md("---\nname: x\nno end", "fb");
    assert_eq!(n, "fb");
    assert!(b.contains("no end"));
}

#[test]
fn a_folder_with_its_own_skill_md_is_one_skill_and_a_pack_yields_each() {
    let one = tempfile::tempdir().unwrap();
    write(one.path(), "SKILL.md", "---\nname: solo\ndescription: d\n---\nbody");
    write(one.path(), "nested/SKILL.md", "---\nname: ignored\ndescription: d\n---\nx");
    let s = scan_dir(one.path()).unwrap();
    assert_eq!(s.candidates.len(), 1);
    assert_eq!((s.candidates[0].key.as_str(), s.candidates[0].name.as_str()), ("", "solo"));

    let pack = tempfile::tempdir().unwrap();
    write(pack.path(), "skills/b/SKILL.md", "---\nname: b\ndescription: d\n---\nB");
    write(pack.path(), "skills/a/SKILL.md", "---\ndescription: d\n---\nA");
    write(pack.path(), ".git/x/SKILL.md", "---\nname: hidden\n---\nH");
    write(pack.path(), "node_modules/m/SKILL.md", "---\nname: dep\n---\nD");
    let s = scan_dir(pack.path()).unwrap();
    let got: Vec<(&str, &str)> = s.candidates.iter().map(|c| (c.key.as_str(), c.name.as_str())).collect();
    // `a` has no name field, so its folder name stands in.
    assert_eq!(got, vec![("skills/a", "a"), ("skills/b", "b")]);
}

#[test]
fn a_single_markdown_file_is_a_skill_and_an_oversize_one_is_skipped_with_a_reason() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "triage.md", "---\nname: triage\ndescription: t\n---\nT");
    let s = scan_dir(&d.path().join("triage.md")).unwrap();
    assert_eq!(s.candidates[0].name, "triage");

    write(d.path(), "big/SKILL.md", &"x".repeat(MAX_FILE_BYTES as usize + 1));
    let s = scan_dir(&d.path().join("big")).unwrap();
    assert!(s.candidates.is_empty());
    assert!(s.warnings[0].contains("skipped"), "{:?}", s.warnings);
}

#[test]
fn only_public_https_sources_and_plain_ref_names_are_accepted() {
    assert!(https_url("https://example.com/a/SKILL.md").is_ok());
    assert!(https_url("https://8.8.8.8/x").is_ok() && https_url("https://[2606:4700::1111]/x").is_ok());
    for bad in [
        "http://example.com/x",
        "file:///etc/passwd",
        "ext::sh -c id",
        "https://user:pw@example.com/x",
        "https://localhost/x",
        "https://127.0.0.1/x",
        "https://10.0.0.5/x",
        "https://169.254.169.254/x",
        "https://[::1]/x",
        "https://[::ffff:127.0.0.1]/x",
        "https://[::ffff:10.0.0.1]/x",
        "https://100.64.0.1/x",
        "https://0.0.0.0/x",
        "https://255.255.255.255/x",
        "https://224.0.0.1/x",
        "https://[fe80::1]/x",
        "https://[fd00::1]/x",
        "https://[64:ff9b::7f00:1]/x",
        "https://foo.localhost/x",
        "not a url",
    ] {
        assert!(https_url(bad).is_err(), "{bad} must be refused");
    }
    assert!(valid_git_ref("main") && valid_git_ref("release/1.2") && valid_git_ref("v1.0.0"));
    assert!(!valid_git_ref("--upload-pack=x") && !valid_git_ref("a..b") && !valid_git_ref("a b") && !valid_git_ref(""));
}

#[test]
fn a_root_skill_without_a_name_takes_the_name_it_is_given_and_a_cut_off_pack_says_so() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "SKILL.md", "---\ndescription: d\n---\nbody");
    let s = scan_dir_named(d.path(), "from-the-repo").unwrap();
    assert_eq!(s.candidates[0].name, "from-the-repo");

    let pack = tempfile::tempdir().unwrap();
    for i in 0..=MAX_CANDIDATES {
        write(pack.path(), &format!("s{i:03}/SKILL.md"), "---\nname: x\ndescription: d\n---\nb");
    }
    let s = scan_dir(pack.path()).unwrap();
    assert_eq!(s.candidates.len(), MAX_CANDIDATES);
    assert!(s.warnings.iter().any(|w| w.contains("only the first")), "{:?}", s.warnings);
}
