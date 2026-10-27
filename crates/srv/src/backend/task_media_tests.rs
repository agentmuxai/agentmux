// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const SHA: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

fn png_bytes() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255])).write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn what_a_file_is_comes_from_its_content_not_its_name() {
    assert_eq!(sniff(&png_bytes()).unwrap().mime, "image/png");
    assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest").unwrap().ext, "jpg");
    assert_eq!(sniff(b"GIF89a....").unwrap().kind, media_kind::IMAGE);
    assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap().mime, "image/svg+xml");
    assert_eq!(sniff(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0]).unwrap().kind, media_kind::RECORDING);
    assert_eq!(sniff(b"\0\0\0\x18ftypisom\0\0\0\0").unwrap().mime, "video/mp4");
    assert_eq!(sniff(b"\0\0\0\x14ftypqt  \0\0\0\0").unwrap().mime, "video/quicktime");
    // What the name says does not matter: text, a PDF and an executable are refused.
    assert!(sniff(b"just some text, called shot.png").is_none());
    assert!(sniff(b"%PDF-1.7 ...").is_none());
    assert!(sniff(b"MZ\x90\0\x03").is_none());
    // A HEIC picture is not one the panes all show.
    assert!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0").is_none());
}

#[test]
fn the_folder_goes_with_the_work_store_so_two_stores_never_share_one() {
    let shared = Some(PathBuf::from("/home/u/.agentmux/shared"));
    // The store every channel shares: the shared home.
    assert_eq!(root_beside(None, shared.clone()), Some(PathBuf::from("/home/u/.agentmux/shared/task-media")));
    // A `task dev` build's own store: beside it, never the shared folder its sweep would clear.
    let own = Some(PathBuf::from("/home/u/.agentmux/dev/branch/work-store.db"));
    assert_eq!(root_beside(own, shared), Some(PathBuf::from("/home/u/.agentmux/dev/branch/task-media")));
    assert_eq!(root_beside(None, None), None);
}

#[test]
fn a_hash_that_is_not_one_never_makes_a_path() {
    let root = Path::new("/store");
    assert!(blob_path(root, "../../etc/passwd", "image/png").is_none());
    assert!(blob_path(root, &SHA[..63], "image/png").is_none());
    assert!(blob_path(root, &SHA.to_uppercase(), "image/png").is_none());
    let p = blob_path(root, SHA, "video/webm").unwrap();
    assert!(p.ends_with(format!("ab/{SHA}.webm")), "{p:?}");
}

#[test]
fn a_committed_file_lands_by_its_hash_with_a_thumbnail_and_the_same_bytes_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bytes = png_bytes();
    let found = sniff(&bytes).unwrap();
    for _ in 0..2 {
        let tmp = new_incoming_path(root);
        std::fs::create_dir_all(tmp.parent().unwrap()).unwrap();
        std::fs::write(&tmp, &bytes).unwrap();
        let dest = commit(root, &tmp, SHA, &found).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), bytes);
        assert!(!tmp.exists(), "the upload is moved or dropped");
    }
    assert!(thumb_path(root, SHA).unwrap().exists());
    let shard: Vec<_> = std::fs::read_dir(root.join("ab")).unwrap().flatten().collect();
    assert_eq!(shard.len(), 2, "the file and its thumbnail, once");
}

#[test]
fn sweep_removes_old_unnamed_files_and_leftover_uploads_and_keeps_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let named = format!("{}{}", "cd", &SHA[2..]);
    let (kept, stray) = (blob_path(root, &named, "image/png").unwrap(), blob_path(root, SHA, "image/png").unwrap());
    for p in [&kept, &stray] {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    let old = SystemTime::now() - Duration::from_secs(3 * 60 * 60);
    let set_old = |p: &Path| std::fs::File::options().write(true).open(p).unwrap().set_modified(old).unwrap();
    let leftover = new_incoming_path(root);
    std::fs::create_dir_all(leftover.parent().unwrap()).unwrap();
    std::fs::write(&leftover, b"x").unwrap();
    for p in [&kept, &stray, &leftover] {
        set_old(p);
    }
    let fresh = new_incoming_path(root);
    std::fs::write(&fresh, b"x").unwrap();

    let removed = sweep(root, |sha| sha == named);
    assert_eq!(removed, 2);
    assert!(kept.exists(), "a file a task names stays however old");
    assert!(!stray.exists());
    assert!(!leftover.exists());
    assert!(fresh.exists(), "an upload in flight stays");
}
