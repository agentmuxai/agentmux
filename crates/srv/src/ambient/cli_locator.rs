// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the ambient calls find a provider's CLI. The lookup behind `install.check` and the launch lives in the
//! server layer (`server::cli_handlers::locate_provider_cli`), which registers it here once at startup
//! ([`set_locator`]), so `ambient` does not name `server` (Area54 #1350). Before that registration nothing is found,
//! which the callers treat as any other CLI that is not there.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

/// A lookup of the CLI a launch of `provider_id` would run: its path, or `None`.
pub type Locator = for<'a> fn(&'a str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;

static LOCATOR: OnceLock<Locator> = OnceLock::new();

/// Register the lookup. Later calls change nothing.
pub fn set_locator(f: Locator) {
    let _ = LOCATOR.set(f);
}

/// The CLI path for `provider_id`, or `None` when none is installed or no lookup is registered yet.
pub async fn locate(provider_id: &str) -> Option<String> {
    match LOCATOR.get() {
        Some(f) => f(provider_id).await,
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(id: &str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
        Box::pin(async move { (id == "known").then(|| "/bin/known".to_string()) })
    }

    #[tokio::test]
    async fn it_asks_the_registered_lookup_and_finds_nothing_without_one() {
        // Whether or not another test registered one first, an unknown provider is not found.
        assert_eq!(locate("nope").await, None);
        set_locator(found);
        assert_eq!(locate("known").await.as_deref(), Some("/bin/known"));
        assert_eq!(locate("nope").await, None);
    }
}
