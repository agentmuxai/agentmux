use super::*;
use serde_json::json;

fn floating() -> CommandPaneOpenData {
    CommandPaneOpenData { view: "editor".into(), floating: Some(true), ..Default::default() }
}

#[test]
fn no_placement_is_fine_floating_or_not() {
    assert!(check(&CommandPaneOpenData::default()).is_ok());
    assert!(check(&floating()).is_ok());
}

#[test]
fn a_placement_without_floating_says_what_to_add() {
    for cmd in [
        CommandPaneOpenData { width: Some(json!(900)), ..Default::default() },
        CommandPaneOpenData { spot: Some("left".into()), ..Default::default() },
        CommandPaneOpenData { x: Some(1.0), y: Some(2.0), ..Default::default() },
        CommandPaneOpenData { floating: Some(false), height: Some(json!("50%")), ..Default::default() },
    ] {
        let e = check(&cmd).unwrap_err();
        assert!(e.starts_with("INVALID_ARG") && e.contains("add floating: true"), "{e}");
    }
}

#[test]
fn a_size_is_pixels_or_a_share_of_the_work_area() {
    for ok in [json!(320), json!(900.5), json!("60%"), json!(" 100 % "), json!("1%")] {
        assert!(check(&CommandPaneOpenData { width: Some(ok.clone()), ..floating() }).is_ok(), "{ok}");
    }
    for bad in [json!(0), json!(-5), json!("0%"), json!("101%"), json!("wide"), json!("600"), json!(true), json!(null), json!([1])] {
        let e = check(&CommandPaneOpenData { height: Some(bad.clone()), ..floating() }).unwrap_err();
        assert!(e.starts_with("INVALID_ARG: height"), "{bad}: {e}");
    }
}

#[test]
fn a_spot_is_one_of_the_named_places() {
    for spot in SPOTS {
        assert!(check(&CommandPaneOpenData { spot: Some(spot.into()), ..floating() }).is_ok(), "{spot}");
    }
    let e = check(&CommandPaneOpenData { spot: Some("middle".into()), ..floating() }).unwrap_err();
    assert!(e.contains("center") && e.contains("bottom-left"), "the refusal lists the places: {e}");
}

#[test]
fn x_and_y_go_together_and_win_over_a_spot() {
    for one in [CommandPaneOpenData { x: Some(10.0), ..floating() }, CommandPaneOpenData { y: Some(10.0), ..floating() }] {
        let e = check(&one).unwrap_err();
        assert!(e.contains("x and y go together") && e.contains("win over spot"), "{e}");
    }
    assert!(check(&CommandPaneOpenData { x: Some(10.0), y: Some(20.0), spot: Some("left".into()), ..floating() }).is_ok());
    assert!(check(&CommandPaneOpenData { x: Some(f64::NAN), y: Some(0.0), ..floating() }).is_err());
}

#[test]
fn a_browser_with_no_size_is_1040_by_760_flush_right_and_held_to_ninety_percent() {
    let mut cmd = CommandPaneOpenData { view: "browser".into(), floating: Some(true), ..Default::default() };
    let cap = browser_defaults(&mut cmd);
    assert_eq!((cmd.width, cmd.height, cmd.spot.as_deref()), (Some(json!(1040.0)), Some(json!(760.0)), Some("right")));
    assert_eq!(cap, Some(0.9));
}

#[test]
fn a_browser_keeps_what_the_agent_gave() {
    let mut cmd = CommandPaneOpenData { width: Some(json!("50%")), spot: Some("left".into()), ..floating() };
    let cap = browser_defaults(&mut cmd);
    assert_eq!(cmd.width, Some(json!("50%")));
    assert_eq!(cmd.height, Some(json!(760.0)), "the size it left out takes the default");
    assert_eq!(cmd.spot.as_deref(), Some("left"));
    assert_eq!(cap, None, "a size the agent chose is not held to the default's cap");

    let mut at = CommandPaneOpenData { x: Some(5.0), y: Some(6.0), ..floating() };
    browser_defaults(&mut at);
    assert_eq!(at.spot, None, "an exact position leaves no spot");
}

#[test]
fn the_event_asks_for_an_inactive_window_and_caps_the_cascade() {
    let cmd = CommandPaneOpenData { width: Some(json!(700)), x: Some(1.0), y: Some(2.0), ..floating() };
    let place = place_for(&cmd, "r1".into(), 9, Some(0.9));
    assert_eq!((place.request_id.as_str(), place.x, place.y, place.max_share), ("r1", Some(1.0), Some(2.0), Some(0.9)));
    assert!(!place.activate, "an agent's window never takes the foreground");
    assert_eq!(place.cascade as usize, MAX_FLOATERS);
}

#[tokio::test]
async fn an_answer_reaches_the_open_that_waits_for_it_once() {
    let rx = expect("fp-1");
    let placed = Placed { x: 1.0, y: 2.0, width: 3.0, height: 4.0, spot: None, clamped: false, display_scale: 1.0 };
    assert!(answered("fp-1", Ok(placed.clone())));
    assert_eq!(rx.await.unwrap().unwrap(), placed);
    assert!(!answered("fp-1", Err("late".into())), "an id is answered once");
    assert!(!answered("fp-never", Err("nobody asked".into())));
}

#[tokio::test]
async fn a_forgotten_wait_takes_no_answer() {
    let _rx = expect("fp-2");
    forget("fp-2");
    assert!(!answered("fp-2", Err("too late".into())));
}
