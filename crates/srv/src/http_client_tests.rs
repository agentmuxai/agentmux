// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A machine with no system certificates is made by pointing `SSL_CERT_FILE` at an empty file: reqwest's loader reads
//! only what that variable names when it is set. The environment is read when a client is built, so these tests run
//! in a child process (the test binary again, one test, the variables set) rather than change this process's. They run
//! on Linux only: on Windows and macOS the machine's certificates come from the system store, which that variable does
//! not name.

#[cfg(target_os = "linux")]
use std::net::SocketAddr;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

use super::*;

/// Set by the parent test in the child's environment: the folder holding the fixture's certificate.
#[cfg(target_os = "linux")]
const CHILD_DIR: &str = "AGENTMUX_HTTP_CLIENT_TEST_DIR";
#[cfg(target_os = "linux")]
const FIXTURE_HOST: &str = "agentmux-viewer";

/// Runs the test named `child` (by its path in this module) in a child process with `SSL_CERT_FILE` set to `cert_file`,
/// and checks that it ran and passed.
#[cfg(target_os = "linux")]
fn run_child(child: &str, cert_file: &Path, dir: &Path) {
    let name = format!("{}::{child}", module_path!().split_once("::").map_or("", |(_, rest)| rest));
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name.as_str(), "--exact", "--test-threads=1"])
        .env("SSL_CERT_FILE", cert_file)
        .env_remove("SSL_CERT_DIR")
        .env(CHILD_DIR, dir)
        .output()
        .unwrap();
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the child test {name} failed:\n{text}");
    assert!(text.contains("1 passed"), "the child test {name} did not run:\n{text}");
}

#[cfg(target_os = "linux")]
fn child_dir() -> Option<PathBuf> {
    std::env::var_os(CHILD_DIR).map(PathBuf::from)
}

/// An https server on the loopback with the fixture certificate, answering every request with "ok".
#[cfg(target_os = "linux")]
async fn tls_server(dir: &Path) -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let tls = crate::backend::viewer::cert::load_or_create(dir).unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(tls.server_config().unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(sock).await else { return };
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

/// A plain http server on the loopback answering every request with "ok".
#[cfg(target_os = "linux")]
async fn http_server() -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
            });
        }
    });
    addr
}

#[cfg(target_os = "linux")]
fn pem(der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let lines: Vec<&str> = b64.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
}

#[cfg(target_os = "linux")]
#[test]
fn a_machine_with_no_certificates_gets_clients_that_trust_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.pem");
    std::fs::write(&empty, "").unwrap();
    run_child("child_with_no_certificates", &empty, dir.path());
}

#[cfg(target_os = "linux")]
#[test]
fn a_machine_with_a_certificate_trusts_it_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let tls = crate::backend::viewer::cert::load_or_create(dir.path()).unwrap();
    let bundle = dir.path().join("bundle.pem");
    std::fs::write(&bundle, pem(&tls.cert_der)).unwrap();
    run_child("child_with_a_certificate", &bundle, dir.path());
}

/// The child of `a_machine_with_no_certificates_gets_clients_that_trust_nothing`; passes at once when run on its own.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn child_with_no_certificates() {
    let Some(dir) = child_dir() else { return };

    // The machine really has nothing, and reqwest's own constructor is the panic this module exists to avoid.
    let Trust::Nothing { reason } = probe() else { panic!("the fixture machine has certificates") };
    assert!(reason.contains("No CA certificates were loaded from the system"), "unexpected reason: {reason}");
    assert!(std::panic::catch_unwind(reqwest::Client::new).is_err(), "reqwest::Client::new() was expected to panic here");

    // Every way srv builds a client works, and says what it found once.
    assert!(no_system_certs());
    assert!(builder().timeout(std::time::Duration::from_secs(5)).build().is_ok());
    let client = client();

    // Plain http to the local machine is unaffected.
    let plain = http_server().await;
    let body = client.get(format!("http://{plain}/")).send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "ok");

    // https fails closed: the server's certificate is not trusted, and the error says there are no system certificates.
    let secure = tls_server(&dir).await;
    let host_client = builder().resolve(FIXTURE_HOST, secure).build().unwrap();
    let url = format!("https://{FIXTURE_HOST}:{}/", secure.port());
    let err = host_client.get(&url).send().await.expect_err("an https request must fail with no certificates");
    let said = explain(&err);
    assert!(said.contains("no system certificates found") && said.contains("ca-certificates"), "the error did not say why: {said}");
    assert!(said.contains("invalid peer certificate"), "the cause is missing from: {said}");

    // Nothing relaxes verification: the same server is trusted only when its certificate is given explicitly.
    let tls = crate::backend::viewer::cert::load_or_create(&dir).unwrap();
    let cert = reqwest::Certificate::from_der(&tls.cert_der).unwrap();
    let with_cert = builder().resolve(FIXTURE_HOST, secure).tls_certs_merge([cert]).build().unwrap();
    assert_eq!(with_cert.get(&url).send().await.unwrap().text().await.unwrap(), "ok");
}

#[cfg(target_os = "linux")]
#[test]
fn certificates_that_vanish_after_start_leave_every_site_with_a_client_that_trusts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let tls = crate::backend::viewer::cert::load_or_create(dir.path()).unwrap();
    let bundle = dir.path().join("bundle.pem");
    std::fs::write(&bundle, pem(&tls.cert_der)).unwrap();
    run_child("child_whose_certificates_vanish", &bundle, dir.path());
}

/// The child of `certificates_that_vanish_after_start_leave_every_site_with_a_client_that_trusts_nothing`: the machine
/// has a certificate when the server starts, and the bundle is replaced by an empty one while it runs.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn child_whose_certificates_vanish() {
    let Some(dir) = child_dir() else { return };
    assert_eq!(*trust(), Trust::System);
    let secure = tls_server(&dir).await;
    let plain = http_server().await;
    let url = format!("https://{FIXTURE_HOST}:{}/", secure.port());

    let empty = dir.join("empty.pem");
    std::fs::write(&empty, "").unwrap();
    std::env::set_var("SSL_CERT_FILE", &empty);

    // A site's own builder can no longer build, which is the panic the sites that cannot return an error had.
    assert!(builder().build().is_err(), "the fixture's certificates were expected to be gone");
    // Its finisher still gives it a client, with the site's own settings (here the timeout and the redirect rule).
    let client = client_with(|b| b.resolve(FIXTURE_HOST, secure).timeout(std::time::Duration::from_secs(5)).redirect(reqwest::redirect::Policy::none()));
    assert_eq!(client.get(format!("http://{plain}/")).send().await.unwrap().text().await.unwrap(), "ok");
    let err = client.get(&url).send().await.expect_err("https must fail closed once the certificates are gone");
    assert!(format!("{err:?}").contains("UnknownIssuer"), "unexpected error: {err:?}");
}

/// The child of `a_machine_with_a_certificate_trusts_it_as_before`; passes at once when run on its own.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn child_with_a_certificate() {
    let Some(dir) = child_dir() else { return };
    assert_eq!(probe(), Trust::System);
    assert!(!no_system_certs());
    let secure = tls_server(&dir).await;
    let url = format!("https://{FIXTURE_HOST}:{}/", secure.port());
    let ok = builder().resolve(FIXTURE_HOST, secure).build().unwrap().get(&url).send().await.unwrap();
    assert_eq!(ok.text().await.unwrap(), "ok", "the machine's certificate was not trusted");
}

#[tokio::test]
async fn an_error_that_is_not_a_certificate_failure_is_left_as_it_is() {
    // Nothing listens on port 1: the connection is refused before any certificate is looked at.
    let err = reqwest::Client::builder().build().unwrap().get("https://127.0.0.1:1/").send().await.unwrap_err();
    assert_eq!(explain_with(&err, true), err.to_string());
    assert_eq!(explain_with(&err, false), err.to_string());
}
