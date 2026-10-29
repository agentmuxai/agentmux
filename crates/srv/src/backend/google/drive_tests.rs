// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn q<'a>(req: &'a GoogleRequest, k: &str) -> Option<&'a str> {
    req.query.iter().find(|(name, _)| *name == k).map(|(_, v)| v.as_str())
}

fn args(file_id: Option<&str>, query: Option<&str>) -> DriveArgs {
    DriveArgs { file_id: file_id.map(str::to_string), query: query.map(str::to_string), ..Default::default() }
}

#[test]
fn a_search_asks_for_untrashed_files_matching_the_words_and_escapes_them() {
    let req = plan("search_files", &args(None, Some("it's a \\ plan"))).unwrap();
    assert_eq!(req.url, "https://www.googleapis.com/drive/v3/files");
    assert_eq!(q(&req, "q"), Some("fullText contains 'it\\'s a \\\\ plan' and trashed = false"));
    assert_eq!(q(&req, "pageSize"), Some("25"));
    assert_eq!(q(&req, "orderBy"), None, "a text search keeps Google's relevance order");
    let all = plan("search_files", &DriveArgs::default()).unwrap();
    assert_eq!(q(&all, "q"), Some("trashed = false"));
    assert_eq!(q(&all, "orderBy"), Some("modifiedTime desc"));
    assert!(!req.text);
}

#[test]
fn the_page_size_is_capped_and_a_page_token_passed_on() {
    let a = DriveArgs { max_results: Some(5000), page_token: Some(" tok ".into()), ..Default::default() };
    let req = plan("search_files", &a).unwrap();
    assert_eq!(q(&req, "pageSize"), Some("100"));
    assert_eq!(q(&req, "pageToken"), Some("tok"));
}

#[test]
fn a_file_action_needs_a_plain_file_id() {
    let e = plan("get_file_metadata", &DriveArgs::default()).unwrap_err();
    assert!(e.contains("needs file_id"), "{e}");
    for bad in ["../about", "a/b", "id with space", "x?alt=media"] {
        assert!(plan("read_file_content", &args(Some(bad), None)).is_err(), "{bad}");
    }
    let req = plan("get_file_metadata", &args(Some("1AbC_d-9"), None)).unwrap();
    assert_eq!(req.url, "https://www.googleapis.com/drive/v3/files/1AbC_d-9");
    assert!(q(&req, "fields").unwrap().contains("mimeType"));
    let e = plan("share_file", &DriveArgs::default()).unwrap_err();
    assert!(e.contains("one of search_files"), "{e}");
}

#[test]
fn google_documents_are_exported_and_plain_text_is_downloaded() {
    let doc = json!({ "id": "d1", "name": "Plan", "mimeType": "application/vnd.google-apps.document" });
    let req = content_plan(&doc).unwrap();
    assert_eq!(req.url, "https://www.googleapis.com/drive/v3/files/d1/export");
    assert_eq!(q(&req, "mimeType"), Some("text/plain"));
    assert!(req.text);

    let sheet = json!({ "id": "s1", "name": "Budget", "mimeType": "application/vnd.google-apps.spreadsheet" });
    assert_eq!(q(&content_plan(&sheet).unwrap(), "mimeType"), Some("text/csv"));

    let md = json!({ "id": "m1", "name": "notes.md", "mimeType": "text/markdown" });
    let req = content_plan(&md).unwrap();
    assert_eq!(req.url, "https://www.googleapis.com/drive/v3/files/m1");
    assert_eq!(q(&req, "alt"), Some("media"));
}

#[test]
fn a_file_that_is_not_text_says_so() {
    for (mime, name) in [("application/pdf", "a.pdf"), ("image/png", "b.png"), ("application/vnd.google-apps.folder", "dir"), ("", "x")] {
        let e = content_plan(&json!({ "id": "f", "name": name, "mimeType": mime })).unwrap_err();
        assert!(e.contains(name) && e.contains("can't be read as text"), "{e}");
    }
}

#[test]
fn files_are_trimmed_to_what_an_agent_needs() {
    let body = json!({
        "nextPageToken": "next",
        "files": [{
            "id": "f1", "name": "Plan", "mimeType": "text/plain", "modifiedTime": "2026-10-28T10:00:00Z", "size": "12",
            "webViewLink": "https://drive.google.com/file/d/f1/view", "owners": [{ "emailAddress": "me@example.com" }],
            "somethingElse": "dropped"
        }]
    });
    let out = shape("search_files", &body);
    assert_eq!(out["next_page_token"], "next");
    assert_eq!(out["files"][0]["owner"], "me@example.com");
    assert_eq!(out["files"][0]["mime_type"], "text/plain");
    assert!(out["files"][0].get("somethingElse").is_none());
    assert_eq!(shape("get_file_metadata", &body["files"][0])["name"], "Plan");
}

#[test]
fn content_notes_a_cut_and_a_sheets_first_sheet() {
    let meta = json!({ "id": "s1", "name": "Budget", "mimeType": "application/vnd.google-apps.spreadsheet" });
    let out = shape_content(&meta, &json!({ "text": "a,b\n1,2\n", "truncated": true }));
    assert_eq!(out["content"], "a,b\n1,2\n");
    assert!(out["truncated"].as_str().unwrap().contains("first 200000 bytes"));
    assert!(out["note"].as_str().unwrap().contains("first sheet"));
    let plain = shape_content(&json!({ "id": "m", "name": "n", "mimeType": "text/plain" }), &json!({ "text": "hi", "truncated": false }));
    assert!(plain.get("truncated").is_none() && plain.get("note").is_none());
}

#[test]
fn any_one_drive_scope_is_enough_and_a_calendar_scope_is_not() {
    let scopes = |s: &[&str]| s.iter().map(|x| format!("https://www.googleapis.com/auth/{x}")).collect::<Vec<_>>();
    assert!(granted_for("search_files", &scopes(&["drive.file"])));
    assert!(granted_for("read_file_content", &scopes(&["drive.readonly"])));
    assert!(!granted_for("search_files", &scopes(&["calendar.events"])));
    assert!(!granted_for("search_files", &[]));
}

#[test]
fn the_log_names_the_file_never_the_search_words() {
    assert_eq!(logged_ids("read_file_content", &args(Some(" f1 "), None)), "f1");
    assert_eq!(logged_ids("search_files", &args(None, Some("secret plan"))), "");
}

#[test]
fn shared_drive_files_are_listed_and_read_not_left_out() {
    let list = plan("search_files", &args(None, Some("plan"))).unwrap();
    assert_eq!(q(&list, "supportsAllDrives"), Some("true"));
    assert_eq!(q(&list, "includeItemsFromAllDrives"), Some("true"));
    let meta = plan("get_file_metadata", &args(Some("f1"), None)).unwrap();
    assert_eq!(q(&meta, "supportsAllDrives"), Some("true"));
    let media = content_plan(&json!({ "id": "m1", "mimeType": "text/plain", "name": "n.txt" })).unwrap();
    assert_eq!(q(&media, "supportsAllDrives"), Some("true"));
}
