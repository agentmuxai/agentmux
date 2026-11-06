// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::sync::atomic::AtomicBool;

fn quiet(_: &'static str, _: &str) {}

fn sha_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    for (name, body) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(body).unwrap();
    }
    w.finish().unwrap();
    buf.into_inner()
}

fn tar_gz_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, body) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        // `set_path` refuses `..`; write the name bytes as a hostile archive would.
        h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        h.set_cksum();
        tar.append(&h, *body).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

fn asset_for(url: &'static str, body: &[u8], archive: Archive, exe: &'static str) -> &'static Asset {
    Box::leak(Box::new(Asset {
        os: "test",
        arch: "test",
        url,
        sha256: Box::leak(sha_of(body).into_boxed_str()),
        size: body.len() as u64,
        archive,
        exe_in_archive: exe,
    }))
}

/// A one-shot HTTP server answering every request with `body`.
async fn serve(body: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn spec_at(prefix: &'static str, asset: &'static Asset) -> DownloadSpec {
    DownloadSpec {
        provider_id: "test",
        version: "1.0.0",
        url_prefix: prefix,
        assets: Box::leak(Box::new([Asset {
            os: asset.os,
            arch: asset.arch,
            url: asset.url,
            sha256: asset.sha256,
            size: asset.size,
            archive: asset.archive,
            exe_in_archive: asset.exe_in_archive,
        }])),
    }
}

#[test]
fn every_download_names_a_registered_provider_its_pin_and_a_well_formed_asset_per_platform() {
    for spec in SPECS {
        let p = crate::backend::providers::get_provider(spec.provider_id).expect(spec.provider_id);
        assert_eq!(p.pinned_version, spec.version, "{}: the spec and the provider disagree on the pin", spec.provider_id);
        assert!(p.npm_package.is_empty(), "{}: a download provider has no npm package", spec.provider_id);
        assert!(spec.url_prefix.starts_with("https://"));
        let mut seen = std::collections::HashSet::new();
        for a in spec.assets {
            assert!(seen.insert((a.os, a.arch)), "{}: duplicate {} {}", spec.provider_id, a.os, a.arch);
            assert!(a.url.starts_with(spec.url_prefix), "{}", a.url);
            assert!(a.url.contains(spec.version), "{}: the url names another version", a.url);
            assert_eq!(a.sha256.len(), 64, "{}", a.url);
            assert!(a.sha256.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)), "{}", a.url);
            assert!(a.size > 1_000_000, "{}", a.url);
            if a.archive == Archive::Raw {
                assert!(a.exe_in_archive.is_empty(), "{}: a raw download has no path inside", a.url);
            } else {
                assert!(!a.exe_in_archive.is_empty() && !a.exe_in_archive.contains("..") && !a.exe_in_archive.starts_with('/'));
            }
        }
        // Every platform AgentMux ships on, except where the vendor has no build.
        for (os, arch) in [("windows", "x86_64"), ("macos", "aarch64"), ("macos", "x86_64"), ("linux", "x86_64"), ("linux", "aarch64")] {
            assert!(spec.asset_for(os, arch).is_some(), "{}: no asset for {os} {arch}", spec.provider_id);
        }
    }
}

#[test]
fn only_a_registered_provider_has_a_download_and_goose_and_grok_do() {
    assert!(download_spec("goose").is_some() && download_spec("grok").is_some());
    for id in ["claude", "../goose", "", "goose "] {
        assert!(download_spec(id).is_none(), "{id:?}");
    }
}

#[test]
fn the_program_lives_in_bin_with_exe_on_windows_only() {
    let bin = download_bin(Path::new("d"), "goose");
    assert_eq!(bin.parent().unwrap(), Path::new("d").join("bin"));
    assert_eq!(bin.file_name().unwrap().to_string_lossy(), if cfg!(windows) { "goose.exe" } else { "goose" });
}

#[test]
fn a_short_download_and_a_wrong_hash_are_refused_and_name_the_expected_hash() {
    let body = b"hello world";
    let a = asset_for("https://example.invalid/x", body, Archive::Zip, "x");
    assert!(verify_download(body.len() as u64, &Sha256::digest(body), a).is_ok());
    assert!(matches!(verify_download(3, &Sha256::digest(body), a), Err(DownloadError::Short { got: 3, .. })));
    let err = verify_download(body.len() as u64, &Sha256::digest(b"other"), a).unwrap_err();
    assert!(matches!(err, DownloadError::HashMismatch { .. }));
    assert!(err.to_string().contains(a.sha256), "the message names the expected hash: {err}");
}

#[tokio::test]
async fn a_download_that_matches_is_kept_and_one_that_does_not_is_refused() {
    let body = b"the pinned release file".to_vec();
    let base = leak(serve(body.clone()).await);
    let url = leak(format!("{base}file.zip"));
    let good = asset_for(url, &body, Archive::Zip, "x");
    let spec = spec_at(base, good);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("download.part");
    let stop = AtomicBool::new(false);

    fetch_verified(good, &dest, &spec, &quiet, &stop).await.expect("matching file");
    assert_eq!(std::fs::read(&dest).unwrap(), body);

    // Same bytes, but the registry holds another hash: refused.
    let wrong = Box::leak(Box::new(Asset { sha256: leak(sha_of(b"something else")), ..clone_asset(good) }));
    let err = fetch_verified(wrong, &dest, &spec, &quiet, &stop).await.unwrap_err();
    assert!(matches!(err, DownloadError::HashMismatch { .. }), "{err}");

    // The registry lists fewer bytes than the server sends: stopped, not kept.
    let small = Box::leak(Box::new(Asset { size: body.len() as u64 - 3, ..clone_asset(good) }));
    let err = fetch_verified(small, &dest, &spec, &quiet, &stop).await.unwrap_err();
    assert!(matches!(err, DownloadError::TooLarge), "{err}");

    // The registry lists more bytes than the server has: truncated.
    let big = Box::leak(Box::new(Asset { size: body.len() as u64 + 5, ..clone_asset(good) }));
    let err = fetch_verified(big, &dest, &spec, &quiet, &stop).await.unwrap_err();
    assert!(matches!(err, DownloadError::Short { .. }), "{err}");
}

fn clone_asset(a: &Asset) -> Asset {
    Asset { os: a.os, arch: a.arch, url: a.url, sha256: a.sha256, size: a.size, archive: a.archive, exe_in_archive: a.exe_in_archive }
}

#[tokio::test]
async fn a_url_outside_the_registered_origin_is_never_fetched() {
    let body = b"x".to_vec();
    let a = asset_for("https://elsewhere.example/file.zip", &body, Archive::Zip, "x");
    let spec = spec_at("https://github.com/vendor/tool/releases/download/v1.0.0/", a);
    let dir = tempfile::tempdir().unwrap();
    let err = fetch_verified(a, &dir.path().join("p"), &spec, &quiet, &AtomicBool::new(false)).await.unwrap_err();
    assert!(err.to_string().contains("outside its registered origin"), "{err}");
}

#[tokio::test]
async fn a_cancelled_download_stops() {
    let body = vec![7u8; 4096];
    let base = leak(serve(body.clone()).await);
    let a = asset_for(leak(format!("{base}f")), &body, Archive::Zip, "x");
    let spec = spec_at(base, a);
    let dir = tempfile::tempdir().unwrap();
    let err = fetch_verified(a, &dir.path().join("p"), &spec, &quiet, &AtomicBool::new(true)).await.unwrap_err();
    assert!(matches!(err, DownloadError::Cancelled), "{err}");
}

#[test]
fn the_executable_is_taken_from_a_zip_and_nothing_else_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("a.zip");
    std::fs::write(&zip, zip_with(&[("goose-package/", b""), ("goose-package/goose.exe", b"MZ-program"), ("goose-package/other.dll", b"dll")])).unwrap();
    let out = dir.path().join("bin").join("goose.exe");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    extract_exe(&zip, Archive::Zip, "goose-package/goose.exe", &out).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"MZ-program");
    let names: Vec<_> = std::fs::read_dir(out.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 1, "only the program is written: {names:?}");
}

/// A probe must be able to turn a CLI's self-update off: the child sees the extra environment.
#[test]
fn the_install_probe_passes_its_environment_to_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let prog = if cfg!(windows) {
        let p = dir.path().join("p.cmd");
        std::fs::write(&p, "@echo off\r\necho %AGM_PROBE_VAR%\r\n").unwrap();
        p
    } else {
        let p = dir.path().join("p");
        std::fs::write(&p, "#!/bin/sh\necho \"$AGM_PROBE_VAR\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    };
    let got = crate::backend::tool_store::probe_version_with_env(&prog.to_string_lossy(), &Some("--version".into()), &[("AGM_PROBE_VAR", "9.8.7")]);
    assert_eq!(got.as_deref(), Some("9.8.7"));
}

#[test]
fn the_providers_that_update_themselves_get_their_switch_on_the_install_probe() {
    for (id, var) in [("antigravity", "AGY_CLI_DISABLE_AUTO_UPDATE"), ("muse", "MUSE_NO_AUTO_UPDATE"), ("grok", "GROK_DISABLE_AUTOUPDATER")] {
        let p = crate::backend::providers::get_provider(id).unwrap();
        assert!(p.auth_extra_env.iter().any(|(k, _)| *k == var), "{id} does not set {var}");
    }
}

#[test]
fn a_raw_download_is_the_program_and_nothing_else_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("download.part");
    std::fs::write(&src, b"ELF-program").unwrap();
    let out = dir.path().join("bin").join("grok");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    extract_exe(&src, Archive::Raw, "", &out).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"ELF-program");
    let names: Vec<_> = std::fs::read_dir(out.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 1, "only the program is written: {names:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&out).unwrap().permissions().mode() & 0o111, 0o111, "runnable");
    }
}

#[test]
fn the_executable_is_taken_from_a_tar_gz_whether_or_not_the_name_starts_with_dot_slash() {
    for name in ["./goose", "goose"] {
        let dir = tempfile::tempdir().unwrap();
        let tgz = dir.path().join("a.tar.gz");
        std::fs::write(&tgz, tar_gz_with(&[("./other", b"o"), (name, b"ELF-program")])).unwrap();
        let out = dir.path().join("goose");
        extract_exe(&tgz, Archive::TarGz, "goose", &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"ELF-program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&out).unwrap().permissions().mode() & 0o111, 0o111, "runnable");
        }
    }
}

#[test]
fn an_archive_without_the_program_fails_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let tgz = dir.path().join("a.tar.gz");
    std::fs::write(&tgz, tar_gz_with(&[("readme", b"x")])).unwrap();
    let out = dir.path().join("goose");
    let err = extract_exe(&tgz, Archive::TarGz, "goose", &out).unwrap_err();
    assert!(err.to_string().contains("not in the archive"), "{err}");
    assert!(!out.exists() && !out.with_extension("part").exists());
}

#[test]
fn an_entry_that_climbs_out_of_the_folder_is_never_written_anywhere() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("inst").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tgz = dir.path().join("a.tar.gz");
    std::fs::write(&tgz, tar_gz_with(&[("../../evil", b"bad"), ("goose", b"ok")])).unwrap();
    extract_exe(&tgz, Archive::TarGz, "goose", &bin.join("goose")).unwrap();
    assert!(!dir.path().join("evil").exists() && !dir.path().join("inst").join("evil").exists());
    assert_eq!(std::fs::read(bin.join("goose")).unwrap(), b"ok");
}

#[test]
fn a_truncated_archive_fails_with_a_reason_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = tar_gz_with(&[("goose", &[1u8; 100_000])]);
    bytes.truncate(bytes.len() / 2);
    let tgz = dir.path().join("a.tar.gz");
    std::fs::write(&tgz, bytes).unwrap();
    let out = dir.path().join("goose");
    assert!(extract_exe(&tgz, Archive::TarGz, "goose", &out).is_err());
    assert!(!out.exists() && !out.with_extension("part").exists());
    std::fs::write(&tgz, b"not an archive").unwrap();
    assert!(extract_exe(&tgz, Archive::Zip, "goose", &out).is_err());
}

#[tokio::test]
async fn a_download_that_does_not_run_is_not_published() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("goose").join("1.0.0");
    // A program that is not a program: right hash, so it unpacks, but `--version` finds nothing.
    let exe = if cfg!(windows) { "goose-package/goose.exe" } else { "goose" };
    let body = if cfg!(windows) { zip_with(&[(exe, b"not an exe")]) } else { tar_gz_with(&[(exe, b"not an exe")]) };
    let archive = if cfg!(windows) { Archive::Zip } else { Archive::TarGz };
    let base = leak(serve(body.clone()).await);
    let a = asset_for(leak(format!("{base}f")), &body, archive, exe);
    let spec = spec_at(base, a);
    let err = install_asset(&dir, &spec, a, "goose", &quiet, &AtomicBool::new(false)).await.unwrap_err();
    assert!(matches!(err, DownloadError::NotRunnable(_)), "{err}");
    assert!(!cli_install::is_complete(&dir), "an install that does not run is never marked complete");
}
