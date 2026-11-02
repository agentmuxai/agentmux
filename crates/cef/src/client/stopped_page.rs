// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The page a browser pane shows when the host stopped its very first
//! navigation (or a redirect on the way to it) and there is no document to
//! leave in place (#18229: a sign-in link opened in a limited pane ended on a
//! grey pane titled "Browser"). The pane's banner asks the person about the
//! address; this page is what stays behind it, so a stopped navigation never
//! ends in a blank pane. It offers Reload and Open in system browser.
//!
//! Like the load-error page it is a `data:` URI carrying the address that was
//! stopped (`navigation::show_load_error_page`'s marker), so the address bar
//! shows that address and a popup from this page is recognised as ours.

use super::helpers::{html_escape, js_string_literal};

/// Is the pane's main frame without a document of its own? Nothing has
/// committed (`current` is empty or `about:blank`), or all it shows is the
/// address that was just stopped: a stopped address never loads, so a frame
/// that reports it has nothing real in it.
pub(crate) fn has_no_document(current: &str, stopped: &str) -> bool {
    let current = current.trim();
    current.is_empty() || current.eq_ignore_ascii_case("about:blank") || current == stopped
}

/// The page, for `stopped` (the address the host would not load) and `sites`
/// (the pane's list, in the normal form `https://example.com`).
pub(crate) fn render_stopped_html(stopped: &str, sites: &[String]) -> String {
    let url = html_escape(stopped);
    let target = js_string_literal(stopped);
    let list = if sites.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = sites.iter().map(|s| format!("<code>{}</code>", html_escape(s))).collect();
        format!("<p>This pane is limited to: {}.</p>", names.join(", "))
    };
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Stopped by AgentMux</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1e1e2e; color: #cdd6f4; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }}
        .box {{ text-align: center; max-width: 600px; padding: 40px; }}
        h1 {{ color: #f9e2af; font-size: 24px; }}
        p {{ color: #a6adc8; line-height: 1.6; }}
        code {{ background: #313244; padding: 2px 8px; border-radius: 4px; font-size: 14px; word-break: break-all; }}
        button, a.button {{ display: inline-block; margin: 20px 6px 0; padding: 10px 24px; border: none; border-radius: 6px; cursor: pointer; font-size: 14px; text-decoration: none; background: #89b4fa; color: #1e1e2e; }}
        a.button {{ background: #313244; color: #cdd6f4; }}
    </style>
</head>
<body>
    <div class="box">
        <h1>AgentMux stopped this page</h1>
        <p>The page tried to go to <code>{url}</code>, which is not one of the sites this pane was opened for.</p>
        {list}
        <p>Answer the question above the page to allow it, or open it yourself.</p>
        <button id="reload">Reload</button>
        <a class="button" href="{url}" target="_blank" rel="noopener noreferrer">Open in system browser</a>
    </div>
    <script>
        // This page is a data: URI, so location.reload() would reload it; go to the real address.
        var __amxTarget = {target};
        document.getElementById("reload").addEventListener("click", function () {{ location.href = __amxTarget; }});
    </script>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_with_nothing_real_in_it_has_no_document() {
        let stopped = "https://admin.microsoft.com/?source=tcemail";
        assert!(has_no_document("", stopped));
        assert!(has_no_document("about:blank", stopped));
        assert!(has_no_document(stopped, stopped));
        assert!(!has_no_document("https://mail.google.com/mail/u/0/", stopped));
    }

    #[test]
    fn the_page_offers_reload_and_the_system_browser_and_names_the_address() {
        let html = render_stopped_html("https://admin.microsoft.com/?source=tcemail", &["https://*.google.com".to_string()]);
        assert!(html.contains("<title>Stopped by AgentMux</title>"));
        assert!(html.contains(">Reload</button>"));
        assert!(html.contains(r#"var __amxTarget = "https://admin.microsoft.com/?source=tcemail";"#));
        assert!(html.contains("Open in system browser"));
        assert!(html.contains(r#"href="https://admin.microsoft.com/?source=tcemail""#));
        assert!(html.contains("https://*.google.com"));
    }

    #[test]
    fn an_address_cannot_break_out_of_the_page() {
        let html = render_stopped_html(r#"https://x.example/"><script>alert(1)</script>'"#, &[r#"<b>"#.to_string()]);
        assert!(!html.contains("<script>alert(1)"));
        assert!(!html.contains("<code><b>"));
        // The Reload handler takes the address as a JS string, not as markup.
        assert!(html.contains(&format!("var __amxTarget = \"https://x.example/{b}\"{b}{u}3e{b}{u}3cscript", b = '\\', u = "u00")));
    }
}
