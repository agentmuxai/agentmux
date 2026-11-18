// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::m0045_palette_snap::tests::{ctx_for, insert_def, set_color, with_isolated_home};
use super::super::m0045_palette_snap::{snap_profiles, snapped};
use super::snap_profiles_file;
use super::*;
use crate::backend::browser_profiles_store::{read_profiles, write_profiles, BrowserProfile};
use crate::backend::palette::{slot_by_id, slots};
use crate::backend::storage::store::Store;

/// The stored hex of each of the 16 slots before, and the id of the slot it becomes (spec §5.1).
const THE_16: [(&str, &str); 16] = [
    ("#e8777f", "red"),
    ("#e87b64", "red"),
    ("#e28247", "orange"),
    ("#d48d24", "amber"),
    ("#bb9c12", "yellow"),
    ("#9aa932", "yellow"),
    ("#75b255", "green"),
    ("#47b777", "jade"),
    ("#00b89e", "teal"),
    ("#00b4be", "teal"),
    ("#00aedd", "sky"),
    ("#53a3f2", "blue"),
    ("#8297f5", "violet"),
    ("#ae88e7", "violet"),
    ("#ce7cc8", "magenta"),
    ("#e276a0", "rose"),
];

fn hex_of(id: &str) -> &'static str {
    slot_by_id(id).unwrap().hex.as_str()
}

fn profile(id: &str, color: &str) -> BrowserProfile {
    BrowserProfile {
        id: id.into(),
        name: id.into(),
        color: color.into(),
        created_at: 0,
        agents_allowed: false,
        owner_agent: None,
    }
}

#[test]
fn each_of_the_sixteen_becomes_the_hex_of_its_new_slot() {
    for (old, now) in THE_16 {
        assert_eq!(snapped(old).as_deref(), Some(hex_of(now)), "{old}");
        assert_eq!(snapped(&old.to_uppercase()).as_deref(), Some(hex_of(now)), "{old} in upper case");
    }
}

#[test]
fn one_of_the_fourteen_is_left_alone_black_and_white_among_them() {
    for slot in slots() {
        assert_eq!(snapped(&slot.hex), None, "{}", slot.id);
        assert_eq!(snapped(&slot.hex.to_uppercase()), None, "{} in upper case", slot.id);
    }
}

#[test]
fn a_custom_colour_goes_to_its_hue_and_a_grey_to_black_or_white() {
    assert_eq!(snapped("#3b82f6").as_deref(), Some(hex_of("blue")));
    assert_eq!(snapped("#ec4899").as_deref(), Some(hex_of("rose")));
    assert_eq!(snapped("#222222").as_deref(), Some("#000000"));
    assert_eq!(snapped("#eeeeee").as_deref(), Some("#ffffff"));
    assert_eq!(snapped("junk"), None);
}

#[test]
fn profiles_move_once_and_a_second_run_changes_nothing() {
    let mut ps: Vec<BrowserProfile> = THE_16.iter().enumerate().map(|(i, (old, _))| profile(&format!("p-{i}"), old)).collect();
    ps.push(profile("p-black", "#000000"));
    ps.push(profile("p-white", "#ffffff"));
    assert!(snap_profiles(&mut ps));
    for (i, (_, now)) in THE_16.iter().enumerate() {
        assert_eq!(ps[i].color, hex_of(now), "profile {i}");
    }
    assert_eq!((ps[16].color.as_str(), ps[17].color.as_str()), ("#000000", "#ffffff"));
    assert!(!snap_profiles(&mut ps), "a second run changes nothing");
}

#[test]
fn the_profiles_file_moves_and_a_second_run_leaves_it_as_it_is() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("browser-profiles.json");
    write_profiles(&path, &[profile("p-cyan", "#00b4be"), profile("p-white", "#ffffff")]).unwrap();
    snap_profiles_file(&path).unwrap();
    let moved = read_profiles(&path).unwrap();
    assert_eq!(moved[0].color, hex_of("teal"), "Cyan merges into Teal");
    assert_eq!(moved[1].color, "#ffffff");
    let written = std::fs::read_to_string(&path).unwrap();
    snap_profiles_file(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
}

#[test]
fn agent_colours_move_and_a_second_run_changes_nothing() {
    with_isolated_home(|| {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mstore = Store::open(tmp.path()).unwrap();
        let azure = insert_def(&mstore, "azure");
        let indigo = insert_def(&mstore, "indigo");
        let green = insert_def(&mstore, "green");
        let black = insert_def(&mstore, "black");
        let none = insert_def(&mstore, "none");
        set_color(&mstore, &azure, "#53a3f2", 7);
        set_color(&mstore, &indigo, "#8297f5", 7);
        set_color(&mstore, &green, hex_of("green"), 42);
        set_color(&mstore, &black, "#000000", 43);

        M0051Palette12Snap.up(&ctx_for(tmp.path())).unwrap();

        let color = |id: &str| mstore.agent_content_get(id, "ui:color").unwrap().unwrap();
        let moved = color(&azure);
        assert_eq!(moved.content, hex_of("blue"), "Azure is Blue");
        assert_ne!(moved.updated_at, 7);
        assert_eq!(color(&indigo).content, hex_of("violet"), "Indigo merges into Violet, as the table says");
        assert_eq!((color(&green).content.as_str(), color(&green).updated_at), (hex_of("green"), 42), "already one of the 14");
        assert_eq!((color(&black).content.as_str(), color(&black).updated_at), ("#000000", 43), "Black is left alone");
        assert!(mstore.agent_content_get(&none, "ui:color").unwrap().is_none());

        M0051Palette12Snap.up(&ctx_for(tmp.path())).unwrap();
        assert_eq!(color(&azure).updated_at, moved.updated_at, "idempotent");
    });
}

#[test]
fn a_missing_channel_store_is_a_noop() {
    with_isolated_home(|| {
        M0051Palette12Snap.up(&ctx_for(std::path::Path::new("Z:/does/not/exist/objects.db"))).unwrap();
    });
}
