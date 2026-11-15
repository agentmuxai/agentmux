// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pane-open timeline (#47585): what each gap is named, what is counted once, and when a navigation is
//! an open's own first load.

use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// An open with marks at the given offsets (ms from the request), in the order of the phases.
fn open_at(started: u64, handed: u64, committed: u64, loaded: u64) -> Timeline {
    let mut t = Timeline::new(Kind::Open);
    t.mark(Phase::Started, ms(started));
    t.mark(Phase::Handed, ms(handed));
    t.mark(Phase::Committed, ms(committed));
    t.mark(Phase::Loaded, ms(loaded));
    t
}

#[test]
fn an_open_names_each_gap_by_the_phase_that_ends_it() {
    let t = open_at(3, 15, 255, 1055);
    assert_eq!(t.gaps(), vec![("queue", 3), ("create", 12), ("engine", 240), ("load", 800)]);
    assert_eq!(t.total_ms(), 1055);
    assert_eq!(
        t.line("ab12cd34ef567890", "views"),
        "[pane-open] block=ab12cd34 open via=views queue=3ms create=12ms engine=240ms load=800ms total=1055ms slowest=load"
    );
}

#[test]
fn a_stalled_ui_thread_shows_as_queue_and_create() {
    // What #47585 looked like: the create waited seconds for the UI thread, and the create call itself took one.
    let t = open_at(2400, 3500, 3600, 3700);
    assert_eq!(t.gaps()[0], ("queue", 2400));
    assert!(t.line("b", "hwnd").ends_with("slowest=queue"));
    assert!(t.is_slow());
}

#[test]
fn a_navigation_has_response_and_load_only() {
    let mut t = Timeline::new(Kind::Navigate);
    t.mark(Phase::Committed, ms(180));
    t.mark(Phase::Loaded, ms(500));
    assert_eq!(t.gaps(), vec![("response", 180), ("load", 320)]);
    assert!(t.line("blk", "hwnd").starts_with("[pane-open] block=blk nav via=hwnd response=180ms load=320ms total=500ms"));
    assert!(!t.is_slow());
}

#[test]
fn a_phase_counts_the_first_time_it_is_reached() {
    let mut t = Timeline::new(Kind::Open);
    t.mark(Phase::Started, ms(5));
    // The create task retries while it waits for its profile; the first start is the one counted.
    t.mark(Phase::Started, ms(900));
    t.mark(Phase::Loaded, ms(1000));
    assert_eq!(t.gaps(), vec![("queue", 5), ("load", 995)]);
}

#[test]
fn a_phase_never_reached_is_left_out() {
    let mut t = Timeline::new(Kind::Open);
    t.mark(Phase::Handed, ms(20));
    t.mark(Phase::Loaded, ms(50));
    assert_eq!(t.gaps(), vec![("create", 20), ("load", 30)]);
}

#[test]
fn a_finished_open_leaves_the_book_and_is_reported_once() {
    let mut book = Book::default();
    let t0 = Instant::now();
    book.begin(Kind::Open, "p", t0);
    book.mark("p", Phase::Started, t0 + ms(4));
    book.mark("p", Phase::Handed, t0 + ms(10));
    book.mark("p", Phase::Committed, t0 + ms(110));
    let done = book.finish("p", t0 + ms(400)).expect("an open in flight");
    assert_eq!(done.gaps(), vec![("queue", 4), ("create", 6), ("engine", 100), ("load", 290)]);
    assert!(book.finish("p", t0 + ms(500)).is_none(), "a second load of the same page is not another open");
}

#[test]
fn a_navigation_during_an_open_is_the_opens_own_first_load() {
    let mut book = Book::default();
    let t0 = Instant::now();
    book.begin(Kind::Open, "p", t0);
    book.begin(Kind::Navigate, "p", t0 + ms(30));
    book.mark("p", Phase::Committed, t0 + ms(100));
    let done = book.finish("p", t0 + ms(200)).unwrap();
    assert_eq!(done.kind, Kind::Open);
    assert_eq!(done.total_ms(), 200, "measured from the request, not from the navigation");
}

#[test]
fn a_navigation_of_an_open_pane_is_timed_from_its_own_start() {
    let mut book = Book::default();
    let t0 = Instant::now();
    book.begin(Kind::Open, "p", t0);
    book.finish("p", t0 + ms(300));
    book.begin(Kind::Navigate, "p", t0 + ms(10_000));
    let done = book.finish("p", t0 + ms(10_450)).unwrap();
    assert_eq!(done.kind, Kind::Navigate);
    assert_eq!(done.total_ms(), 450);
}

#[test]
fn a_second_navigation_replaces_one_that_never_finished() {
    let mut book = Book::default();
    let t0 = Instant::now();
    book.begin(Kind::Navigate, "p", t0);
    book.begin(Kind::Navigate, "p", t0 + ms(5_000));
    let done = book.finish("p", t0 + ms(5_200)).unwrap();
    assert_eq!(done.total_ms(), 200);
}

#[test]
fn an_open_that_was_lost_does_not_swallow_later_navigations() {
    let mut book = Book::default();
    let t0 = Instant::now();
    book.begin(Kind::Open, "p", t0);
    // The page never loaded and nothing closed the pane; a minute on, the user navigates.
    book.begin(Kind::Navigate, "p", t0 + STALE + ms(1));
    let done = book.finish("p", t0 + STALE + ms(300)).unwrap();
    assert_eq!(done.kind, Kind::Navigate);
}

#[test]
fn a_new_open_replaces_whatever_was_in_flight_and_forget_drops_it() {
    let mut book = Book::default();
    let t0 = Instant::now();
    book.begin(Kind::Open, "p", t0);
    book.begin(Kind::Open, "p", t0 + ms(2_000)); // closed while closing, re-created
    assert_eq!(book.finish("p", t0 + ms(2_100)).unwrap().total_ms(), 100);
    book.begin(Kind::Open, "q", t0);
    book.forget("q");
    assert!(book.finish("q", t0 + ms(10)).is_none());
}

#[test]
fn marks_for_a_pane_with_nothing_in_flight_are_ignored() {
    let mut book = Book::default();
    book.mark("nobody", Phase::Committed, Instant::now());
    assert!(book.finish("nobody", Instant::now()).is_none());
}
