// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::identity::aws_broker::mint::tests::{creds_json, now};
use crate::identity::aws_broker::mint::parse_process_json;

/// A session that expires `mins` minutes after [`now`].
fn session(mins: i64) -> Minted {
    parse_process_json(creds_json(mins).as_bytes(), now()).unwrap()
}

async fn ask(s: &Sessions, agent: &str, reason: &str, at: DateTime<Utc>, mins: i64, minted: &AtomicUsize) -> Result<(Minted, Source), SessionError> {
    s.get_or_mint(agent, "deploy", reason, 12, at, || async {
        minted.fetch_add(1, Ordering::SeqCst);
        Ok(session(mins))
    })
    .await
}

#[tokio::test]
async fn the_same_session_comes_back_while_more_than_ten_minutes_remain() {
    let (s, mints) = (Sessions::default(), AtomicUsize::new(0));
    let (first, source) = ask(&s, "a", "deploy x", now(), 30, &mints).await.unwrap();
    assert_eq!(source, Source::Fresh);
    // Twenty minutes in, ten remain: the margin is "at least ten", so it is still handed out.
    let (again, source) = ask(&s, "a", "deploy x", now() + chrono::Duration::minutes(20), 30, &mints).await.unwrap();
    assert_eq!((source, again.expiration), (Source::Reused, first.expiration));
    assert_eq!(mints.load(Ordering::SeqCst), 1, "one mint for many calls");
}

#[tokio::test]
async fn a_session_with_under_ten_minutes_left_is_not_handed_out_again() {
    let (s, mints) = (Sessions::default(), AtomicUsize::new(0));
    ask(&s, "a", "r", now(), 30, &mints).await.unwrap();
    // botocore refuses credentials in their last ten minutes, so a nine-minute-old answer would fail in the CLI.
    let at = now() + chrono::Duration::minutes(21);
    let (fresh, source) = s.get_or_mint("a", "deploy", "r", 12, at, || async { Ok(session(51)) }).await.unwrap();
    assert_eq!(source, Source::Fresh);
    assert_eq!(fresh.expiration, now() + chrono::Duration::minutes(51));
}

#[tokio::test]
async fn another_reason_another_agent_or_another_grant_is_another_session() {
    let (s, mints) = (Sessions::default(), AtomicUsize::new(0));
    ask(&s, "a", "deploy x", now(), 30, &mints).await.unwrap();
    ask(&s, "a", "deploy y", now(), 30, &mints).await.unwrap();
    ask(&s, "b", "deploy x", now(), 30, &mints).await.unwrap();
    s.get_or_mint("a", "other", "deploy x", 12, now(), || async {
        mints.fetch_add(1, Ordering::SeqCst);
        Ok(session(30))
    })
    .await
    .unwrap();
    assert_eq!(mints.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn requests_at_the_same_moment_share_one_mint() {
    let s = Arc::new(Sessions::default());
    let mints = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let (s, mints) = (s.clone(), mints.clone());
            tokio::spawn(async move {
                s.get_or_mint("a", "deploy", "r", 12, now(), || async {
                    mints.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    Ok(session(30))
                })
                .await
                .map(|(_, source)| source)
            })
        })
        .collect();
    let mut sources = Vec::new();
    for t in tasks {
        sources.push(t.await.unwrap().unwrap());
    }
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(sources.iter().filter(|s| **s == Source::Fresh).count(), 1, "one asked AWS, the others waited for it");
}

#[tokio::test]
async fn an_agent_can_mint_only_so_often_in_an_hour_and_failures_count_too() {
    let s = Sessions::default();
    let fail = |_: ()| async { Err::<Minted, _>(MintError::TookTooLong) };
    for _ in 0..3 {
        let r = s.get_or_mint("a", "deploy", "r", 3, now(), || fail(())).await;
        assert!(matches!(r, Err(SessionError::Mint(MintError::TookTooLong))));
    }
    let r = s.get_or_mint("a", "deploy", "r", 3, now() + chrono::Duration::minutes(59), || fail(())).await;
    assert_eq!(r.unwrap_err(), SessionError::TooMany { cap: 3 }, "a failing loop is bounded too");
    // Another agent is not held up by it; an hour on, this one may try again.
    assert!(s.get_or_mint("b", "deploy", "r", 3, now(), || async { Ok(session(30)) }).await.is_ok());
    assert!(matches!(
        s.get_or_mint("a", "deploy", "r", 3, now() + chrono::Duration::minutes(61), || fail(())).await,
        Err(SessionError::Mint(_))
    ));
}

#[tokio::test]
async fn a_cache_hit_does_not_count_against_the_cap() {
    let (s, mints) = (Sessions::default(), AtomicUsize::new(0));
    for _ in 0..20 {
        s.get_or_mint("a", "deploy", "r", 2, now(), || async {
            mints.fetch_add(1, Ordering::SeqCst);
            Ok(session(30))
        })
        .await
        .unwrap();
    }
    assert_eq!(mints.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_stopped_agents_sessions_are_dropped_and_another_agents_are_not() {
    let (s, mints) = (Sessions::default(), AtomicUsize::new(0));
    ask(&s, "a", "r", now(), 30, &mints).await.unwrap();
    ask(&s, "b", "r", now(), 30, &mints).await.unwrap();
    s.revoke_agent("a");
    assert_eq!((s.held("a"), s.held("b")), (0, 1));
    let (_, source) = ask(&s, "a", "r", now(), 30, &mints).await.unwrap();
    assert_eq!(source, Source::Fresh, "it mints again");
    let (_, source) = ask(&s, "b", "r", now(), 30, &mints).await.unwrap();
    assert_eq!(source, Source::Reused);
}

#[tokio::test]
async fn one_agent_never_gets_another_agents_session() {
    let (s, mints) = (Sessions::default(), AtomicUsize::new(0));
    let (a, _) = ask(&s, "a", "r", now(), 30, &mints).await.unwrap();
    let (b, source) = ask(&s, "b", "r", now(), 45, &mints).await.unwrap();
    assert_eq!(source, Source::Fresh);
    assert_ne!(a.expiration, b.expiration);
}

#[tokio::test]
async fn an_agent_that_varies_its_reason_cannot_grow_the_map_past_what_it_may_mint() {
    let s = Sessions::default();
    // Cap 3: three reasons mint, the other 40 are refused, and the refused ones leave no slot behind.
    for n in 0..43 {
        let _ = s.get_or_mint("a", "deploy", &format!("reason {n}"), 3, now(), || async { Ok(session(30)) }).await;
    }
    assert_eq!(s.held("a"), 3, "only the sessions that exist are held");
}

#[tokio::test]
async fn a_failed_mint_leaves_nothing_behind() {
    let s = Sessions::default();
    for n in 0..5 {
        let r = s.get_or_mint("a", "deploy", &format!("r{n}"), 12, now(), || async { Err::<Minted, _>(MintError::TookTooLong) }).await;
        assert!(r.is_err());
    }
    assert_eq!(s.held("a"), 0);
}

#[tokio::test]
async fn expired_sessions_are_dropped_when_the_map_gets_large() {
    let s = Sessions::default();
    for n in 0..40 {
        s.get_or_mint(&format!("agent-{n}"), "deploy", "r", 12, now(), || async { Ok(session(30)) }).await.unwrap();
    }
    let total = |s: &Sessions| s.slots.lock().unwrap().len();
    assert_eq!(total(&s), 40);
    // Two hours on, every one of them has expired: making a new slot clears the dead ones.
    s.get_or_mint("late", "deploy", "r", 12, now() + chrono::Duration::hours(2), || async { Ok(session(150)) }).await.unwrap();
    assert!(total(&s) <= 2, "{}", total(&s));
}
