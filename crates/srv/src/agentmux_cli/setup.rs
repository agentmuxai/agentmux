// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux setup`: the choices of a first run, as questions in a terminal or as flags. What to do is
//! decided here ([`plan`], with no side effect, so every rule has a test); doing it is `main`'s.
//! A question with a flag is not asked. `--yes` takes the current choice (the defaults on a first
//! run) for every question that has no flag, and with no terminal and no `--yes` setup refuses
//! rather than guess.

use clap::Args;

use crate::settings::{check_port, SecretStore, Settings};

#[derive(Args, Debug, Clone, Default)]
pub struct SetupArgs {
    /// Take the current choice, or the default, for every question without a flag.
    #[arg(long)]
    pub yes: bool,
    /// The web port the browser opens.
    #[arg(long)]
    pub port: Option<u16>,
    /// Where the server keeps secrets.
    #[arg(long, value_enum)]
    pub secret_store: Option<SecretStore>,
    /// The address a browser reaches the server on when it is not the server's own: `http://127.0.0.1:<port>`.
    #[arg(long)]
    pub public_origin: Option<String>,
    /// Start the server at every login (a user service).
    #[arg(long, conflicts_with = "no_service")]
    pub service: bool,
    /// Do not install a service.
    #[arg(long)]
    pub no_service: bool,
    /// Do not start the server now.
    #[arg(long)]
    pub no_start: bool,
    /// Do not offer to sign a browser in.
    #[arg(long)]
    pub no_open: bool,
    /// The server program (default: `agentmux-srv` beside this command).
    #[arg(long, value_name = "FILE")]
    pub srv_bin: Option<std::path::PathBuf>,
    /// The built web UI (default: `web` beside the install's `bin` folder).
    #[arg(long, value_name = "DIR")]
    pub web_dir: Option<std::path::PathBuf>,
}

/// What the person's answers come to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub settings: Settings,
    pub service: bool,
    pub start: bool,
    pub open: bool,
}

/// The person, or a script standing in for them in a test.
pub trait Ask {
    /// Is there a terminal to ask on?
    fn interactive(&self) -> bool;
    fn yes_no(&mut self, question: &str, default: bool) -> bool;
    /// A line of text; an empty answer is the default.
    fn line(&mut self, question: &str, default: &str) -> String;
}

/// What setup would do, from the flags, the current settings and the answers.
pub fn plan(args: &SetupArgs, current: &Settings, service_supported: bool, service_installed: bool, ask: &mut impl Ask) -> Result<Plan, String> {
    if !ask.interactive() && !args.yes {
        return Err("setup asks questions and there is no terminal: add --yes to take the defaults, and give each choice as a flag (--port, --secret-store, --service or --no-service)".into());
    }
    let asking = ask.interactive() && !args.yes;

    let port = match (args.port, asking) {
        (Some(p), _) => p,
        (None, false) => current.port,
        (None, true) => parse_port(&ask.line(&format!("Web port for the browser's address (the next one is used too) [{}]", current.port), &current.port.to_string()))?,
    };
    check_port(port)?;

    let secret_store = match (args.secret_store, asking) {
        (Some(s), _) => s,
        (None, false) => current.secret_store,
        (None, true) => {
            let default = current.secret_store.flag();
            let answer = ask.line(
                &format!("Where should secrets (API keys, connected accounts) live? `file`: owner-only files, for a server with no keychain. `keychain`: this machine's keychain, where the desktop app keeps them [{default}]"),
                default,
            );
            match answer.trim() {
                "file" => SecretStore::File,
                "keychain" => SecretStore::Keychain,
                other => return Err(format!("{other:?} is neither `file` nor `keychain`")),
            }
        }
    };
    if let Some(origin) = &args.public_origin {
        check_public_origin(origin)?;
    }
    let public_origin = args.public_origin.clone().or_else(|| current.public_origin.clone());

    let service = if !service_supported || args.no_service {
        false
    } else if args.service {
        true
    } else if asking {
        ask.yes_no("Start AgentMux at every login (a user service, no root)?", true)
    } else {
        // Not named and not asked: the service stays as it is, so a rerun changes only what it is told to.
        service_installed
    };
    let start = !args.no_start;
    // Signing in needs the person at a terminal to confirm; a script is told the command instead.
    let open = start && !args.no_open && ask.interactive() && (!asking || ask.yes_no("Sign a browser in now?", true));
    Ok(Plan { settings: Settings { port, secret_store, public_origin }, service, start, open })
}

fn parse_port(text: &str) -> Result<u16, String> {
    text.trim().parse::<u16>().map_err(|_| format!("{:?} is not a port number", text.trim()))
}

/// `--public-origin` is held to the rule the server holds it to: `http://127.0.0.1:<port>`.
fn check_public_origin(origin: &str) -> Result<(), String> {
    let rest = origin.strip_prefix("http://127.0.0.1:").ok_or_else(|| format!("--public-origin {origin:?}: must be http://127.0.0.1:<port>"))?;
    rest.parse::<u16>().ok().filter(|p| *p != 0).map(|_| ()).ok_or_else(|| format!("--public-origin {origin:?}: must be http://127.0.0.1:<port>"))
}

/// The person's terminal.
pub struct Terminal;

impl Ask for Terminal {
    fn interactive(&self) -> bool {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    fn yes_no(&mut self, question: &str, default: bool) -> bool {
        let answer = self.line(&format!("{question} [{}]", if default { "Y/n" } else { "y/N" }), "");
        match answer.trim().to_ascii_lowercase().as_str() {
            "" => default,
            "y" | "yes" => true,
            _ => false,
        }
    }

    fn line(&mut self, question: &str, default: &str) -> String {
        use std::io::Write;
        print!("{question}: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(n) if n > 0 && !line.trim().is_empty() => line.trim().to_string(),
            _ => default.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers in order; a question asked more often than answers were given is a test failure.
    struct Script {
        terminal: bool,
        answers: Vec<&'static str>,
        asked: Vec<String>,
    }

    impl Script {
        fn new(terminal: bool, answers: &[&'static str]) -> Script {
            Script { terminal, answers: answers.iter().rev().copied().collect(), asked: Vec::new() }
        }
        fn next(&mut self, question: &str) -> &'static str {
            self.asked.push(question.to_string());
            self.answers.pop().expect("asked more than the script answers")
        }
    }

    impl Ask for Script {
        fn interactive(&self) -> bool {
            self.terminal
        }
        fn yes_no(&mut self, question: &str, default: bool) -> bool {
            match self.next(question) {
                "" => default,
                a => a == "y",
            }
        }
        fn line(&mut self, question: &str, default: &str) -> String {
            match self.next(question) {
                "" => default.to_string(),
                a => a.to_string(),
            }
        }
    }

    fn flags(f: impl FnOnce(&mut SetupArgs)) -> SetupArgs {
        let mut a = SetupArgs::default();
        f(&mut a);
        a
    }

    #[test]
    fn with_no_terminal_and_no_yes_setup_refuses_and_asks_nothing() {
        let mut ask = Script::new(false, &[]);
        let e = plan(&SetupArgs::default(), &Settings::default(), true, false, &mut ask).unwrap_err();
        assert!(e.contains("--yes") && e.contains("no terminal"), "{e}");
        assert!(ask.asked.is_empty());
    }

    #[test]
    fn yes_with_no_terminal_takes_the_defaults_and_leaves_the_service_as_it_is() {
        let mut ask = Script::new(false, &[]);
        let p = plan(&flags(|a| a.yes = true), &Settings::default(), true, false, &mut ask).unwrap();
        assert_eq!(p, Plan { settings: Settings::default(), service: false, start: true, open: false });
        assert!(ask.asked.is_empty(), "nothing is asked");
        let p = plan(&flags(|a| a.yes = true), &Settings::default(), true, true, &mut Script::new(false, &[])).unwrap();
        assert!(p.service, "an installed service is kept by a rerun that does not name it");
        let p = plan(&flags(|a| a.port = Some(7500)), &Settings::default(), true, false, &mut Script::new(false, &[])).unwrap_err();
        assert!(p.contains("--yes"), "{p}");
    }

    #[test]
    fn the_documented_script_line_is_what_it_says() {
        let a = flags(|a| {
            a.yes = true;
            a.port = Some(7380);
            a.secret_store = Some(SecretStore::File);
            a.service = true;
            a.no_open = true;
        });
        let p = plan(&a, &Settings::default(), true, false, &mut Script::new(false, &[])).unwrap();
        assert_eq!((p.settings.port, p.settings.secret_store, p.service, p.start, p.open), (7380, SecretStore::File, true, true, false));
    }

    #[test]
    fn yes_keeps_what_an_earlier_run_chose() {
        let current = Settings { port: 7500, secret_store: SecretStore::Keychain, public_origin: Some("http://127.0.0.1:9000".into()) };
        let p = plan(&flags(|a| a.yes = true), &current, true, false, &mut Script::new(true, &[])).unwrap();
        assert_eq!(p.settings, current);
    }

    #[test]
    fn a_flag_is_not_asked_and_the_rest_are() {
        let mut ask = Script::new(true, &["keychain", "n", ""]);
        let p = plan(&flags(|a| a.port = Some(7400)), &Settings::default(), true, false, &mut ask).unwrap();
        assert_eq!(p.settings.port, 7400);
        assert_eq!(p.settings.secret_store, SecretStore::Keychain);
        assert!(!p.service, "the person said no to the service");
        assert!(p.open, "the person took the default: sign in now");
        assert_eq!(ask.asked.len(), 3, "secret store, service, sign-in: {:?}", ask.asked);
        assert!(ask.asked.iter().all(|q| !q.contains("Web port")), "the port had a flag");
    }

    #[test]
    fn an_empty_answer_keeps_the_current_choice() {
        let current = Settings { port: 7555, ..Settings::default() };
        let mut ask = Script::new(true, &["", "", "", ""]);
        let p = plan(&SetupArgs::default(), &current, true, false, &mut ask).unwrap();
        assert_eq!(p.settings.port, 7555);
        assert_eq!(p.settings.secret_store, SecretStore::File);
        assert!(p.service && p.start && p.open);
    }

    #[test]
    fn a_port_or_store_that_is_not_one_is_refused_before_anything_is_changed() {
        for bad in [80u16, 1023, 65535] {
            let e = plan(&flags(|a| {
                a.yes = true;
                a.port = Some(bad);
            }), &Settings::default(), true, false, &mut Script::new(false, &[])).unwrap_err();
            assert!(e.contains("1024 to 65534"), "{bad}: {e}");
        }
        let e = plan(&SetupArgs::default(), &Settings::default(), true, false, &mut Script::new(true, &["seven thousand"])).unwrap_err();
        assert!(e.contains("not a port number"), "{e}");
        let e = plan(&SetupArgs::default(), &Settings::default(), true, false, &mut Script::new(true, &["", "vault"])).unwrap_err();
        assert!(e.contains("neither"), "{e}");
    }

    #[test]
    fn the_public_origin_is_held_to_the_loopback_rule() {
        for ok in ["http://127.0.0.1:8080", "http://127.0.0.1:1"] {
            let a = flags(|a| {
                a.yes = true;
                a.public_origin = Some(ok.into());
            });
            assert_eq!(plan(&a, &Settings::default(), true, false, &mut Script::new(false, &[])).unwrap().settings.public_origin.as_deref(), Some(ok));
        }
        for bad in ["https://127.0.0.1:8080", "http://localhost:8080", "http://example.com", "http://127.0.0.1", "http://127.0.0.1:0", "http://127.0.0.1:x"] {
            let a = flags(|a| {
                a.yes = true;
                a.public_origin = Some(bad.into());
            });
            assert!(plan(&a, &Settings::default(), true, false, &mut Script::new(false, &[])).is_err(), "{bad}");
        }
    }

    #[test]
    fn where_there_is_no_service_none_is_installed_or_asked_about() {
        let mut ask = Script::new(true, &["", "", ""]);
        let p = plan(&SetupArgs::default(), &Settings::default(), false, false, &mut ask).unwrap();
        assert!(!p.service);
        assert!(ask.asked.iter().all(|q| !q.contains("every login")), "{:?}", ask.asked);
        let p = plan(&flags(|a| a.service = true), &Settings::default(), false, false, &mut Script::new(true, &["", "", ""])).unwrap();
        assert!(!p.service, "--service where there is none installs none");
    }

    #[test]
    fn no_start_means_no_sign_in_offer_and_no_open_means_none_either() {
        let p = plan(&flags(|a| {
            a.yes = true;
            a.no_start = true;
        }), &Settings::default(), true, false, &mut Script::new(true, &[])).unwrap();
        assert!(!p.start && !p.open);
        let p = plan(&flags(|a| {
            a.yes = true;
            a.no_open = true;
        }), &Settings::default(), true, false, &mut Script::new(true, &[])).unwrap();
        assert!(p.start && !p.open);
        let both = SetupArgs::try_parse_from_flags(&["--service", "--no-service"]);
        assert!(both.is_err(), "the two flags exclude each other");
    }

    impl SetupArgs {
        fn try_parse_from_flags(args: &[&str]) -> Result<SetupArgs, clap::Error> {
            #[derive(clap::Parser)]
            struct Wrap {
                #[command(flatten)]
                inner: SetupArgs,
            }
            use clap::Parser;
            Wrap::try_parse_from([&["x"], args].concat()).map(|w| w.inner)
        }
    }
}
