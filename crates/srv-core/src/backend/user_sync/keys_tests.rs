// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A collection's keys between machines with real sealing: a key reaches a joiner sealed to it and opens nowhere else,
// a removal starts a new key that only the machines left are given, a removal that loses a fork leaves no key behind, a
// removal that cannot be given a key is not signed, and a join waiting for a person is still there in the next call.

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::adapter::NewestWins;
use super::chain_store;
use super::hlc::Clock;
use super::keys::{self, KeyStore, SqlKeys, StoreKeys};
use super::log::{self, Outcome, Signer};
use super::members::{Change, Chain, Entry, Policy, Subject};
use super::ops::{self, Ctx, JoinReq, Joined, KeyDelivery, KeysReq, Me, MembersReq, OpError, PendingJoin, Proof};
use super::seal;

const NOW: i64 = 1_800_000_000_000;
const C: &str = "board.k";

struct Yes;
impl Proof for Yes {
    fn auto_proven(&self, _: &str) -> bool {
        true
    }
}

/// A delivery that cannot make a key.
struct Cannot;
impl KeyDelivery for Cannot {
    fn keys_for(&self, _: &str, _: &Subject) -> Vec<(u32, String)> {
        Vec::new()
    }
    fn mint(&self, _: &str, _: u32) -> bool {
        false
    }
}

struct N {
    id: String,
    seed: [u8; 32],
    public: [u8; 32],
    subject: Subject,
    conn: Connection,
    chains: BTreeMap<String, Chain>,
    clock: Clock,
    pending: Vec<PendingJoin>,
}

fn n(seed: u8) -> N {
    let seed = [seed; 32];
    let public = ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let subject = Subject::machine(&seed, &crate::backend::jekt_seal::box_public(&seed).unwrap(), "label").unwrap();
    let conn = Connection::open_in_memory().unwrap();
    log::init(&conn).unwrap();
    N { id: subject.id.clone(), clock: Clock::new(&subject.id), seed, public, subject, conn, chains: BTreeMap::new(), pending: Vec::new() }
}

impl N {
    /// Runs `f` with this machine's real key delivery over its own store.
    fn with<R>(&mut self, f: impl FnOnce(&mut Ctx) -> R) -> R {
        let store = SqlKeys(&self.conn);
        let delivery = StoreKeys { store: &store, wan_public: self.public, wan_seed: self.seed };
        let mut ctx = Ctx {
            conn: &self.conn,
            chains: &mut self.chains,
            clock: &mut self.clock,
            pending: &mut self.pending,
            me: Me { id: &self.id, private: &self.seed },
            proof: &Yes,
            keys: &delivery,
            now_ms: NOW,
            chain_cap: ops::MAX_ENTRIES,
            active: true,
        };
        f(&mut ctx)
    }

    fn found(&mut self, kind: &str, policy: Policy) {
        let store = SqlKeys(&self.conn);
        let delivery = StoreKeys { store: &store, wan_public: self.public, wan_seed: self.seed };
        ops::found(&self.conn, &mut self.chains, &delivery, C, kind, &self.subject, &self.seed, policy, NOW).unwrap();
    }

    fn epochs(&self) -> Vec<u32> {
        SqlKeys(&self.conn).epochs(C)
    }

    fn write(&mut self, record: &str, body: &str) -> super::record::Record {
        let ring = keys::ring(&SqlKeys(&self.conn), C);
        let mut signer = Signer { machine: &self.id, private: &self.seed, clock: &mut self.clock };
        log::append_local_with(&self.conn, &mut signer, C, record, false, NOW, seal::sealer(&ring, body.as_bytes())).unwrap()
    }

    fn open(&self, rec: &super::record::Record) -> Option<Vec<u8>> {
        let ring = keys::ring(&SqlKeys(&self.conn), C);
        seal::open(&ring, &rec.header, &rec.body).ok().map(|b| b.to_vec())
    }
}

/// `joiner` joins `host`'s collection, takes the chain and the keys it is given.
fn join(host: &mut N, joiner: &mut N, kind: &str) -> Vec<(u32, String)> {
    let req = JoinReq { collection: C.into(), subject: joiner.subject.clone() };
    let id = joiner.id.clone();
    let Joined::Added { entries, keys } = host.with(|ctx| ops::join(ctx, &id, &req)).unwrap() else { panic!("added") };
    let host_id = host.id.clone();
    let chain = ops::take_join(C, &joiner.id, entries, &|m| m == host_id).unwrap();
    ops::adopt(&joiner.conn, &mut joiner.chains, C, kind, chain, NOW).unwrap();
    let taken = joiner.with(|ctx| ctx.keys.take(C, &keys));
    assert_eq!(taken, keys.len());
    keys
}

fn catch_up(to: &mut N, from: &mut N) {
    let have = to.chains[C].entries.len() as u64;
    let id = to.id.clone();
    let ans = from.with(|ctx| ops::members(ctx, &id, &MembersReq { collection: C.into(), have_n: have, offered: vec![] })).unwrap();
    let chain = to.chains.get_mut(C).unwrap();
    for e in ans.entries {
        chain.take(C, e).unwrap();
    }
    ops::sync_epoch(&to.conn, C, chain).unwrap();
    chain_store::save(&to.conn, C, chain).unwrap();
}

fn pins_of(n: &N) -> log::Pins {
    n.chains[C].state(C).unwrap().pins()
}

fn take_record(to: &mut N, rec: &super::record::Record) {
    let pins = pins_of(to);
    let out = log::apply_remote(&to.conn, rec, &pins, &mut to.clock, NOW, &NewestWins).unwrap();
    assert!(matches!(out, Outcome::Applied | Outcome::Duplicate), "{out:?}");
}

fn ended(pairs: &[(&N, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(n, s)| (n.id.clone(), *s)).collect()
}

#[test]
fn a_key_reaches_a_joiner_sealed_to_it_and_opens_nowhere_else() {
    let (mut a, mut b, c) = (n(1), n(2), n(3));
    a.found("kanban", Policy::AutoProven);
    assert_eq!(a.epochs(), vec![0], "the founder made the first key");
    let rec = a.write("card", "hello");
    let sealed = join(&mut a, &mut b, "kanban");
    assert_eq!(sealed.len(), 1);
    take_record(&mut b, &rec);
    assert_eq!(b.open(&rec).as_deref(), Some(&b"hello"[..]), "the new member opens what was written before it joined");
    let (epoch, text) = &sealed[0];
    assert!(keys::open_sealed(text, &b.public, &b.seed, C, *epoch).is_some());
    assert!(keys::open_sealed(text, &c.public, &c.seed, C, *epoch).is_none(), "another machine cannot open it");
    assert!(keys::open_sealed(text, &b.public, &b.seed, "board.other", *epoch).is_none(), "nor for another collection");
    assert!(keys::open_sealed(text, &b.public, &b.seed, C, epoch + 1).is_none(), "nor for another epoch");
    let stranger = c.id.clone();
    let ask = KeysReq { collection: C.into(), have: vec![] };
    assert_eq!(a.with(|ctx| ops::keys(ctx, &stranger, &ask)), Err(OpError::NotMember));
}

#[test]
fn a_removal_starts_a_new_key_that_only_the_machines_left_are_given() {
    let (mut a, mut b, mut c) = (n(1), n(2), n(3));
    a.found("kanban", Policy::AutoProven);
    join(&mut a, &mut b, "kanban");
    join(&mut a, &mut c, "kanban");
    catch_up(&mut b, &mut a);
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let cid = c.id.clone();
    a.with(|ctx| ops::change(ctx, C, Change::Remove { id: cid, ended: gone })).unwrap();
    assert_eq!(a.epochs(), vec![0, 1], "the removal made the next key before it was signed");
    let after = a.write("card", "after the removal");
    catch_up(&mut b, &mut a);
    take_record(&mut b, &after);
    assert_eq!(b.open(&after), None, "B has no key for the new epoch yet");
    let (bid, cid) = (b.id.clone(), c.id.clone());
    let ask = KeysReq { collection: C.into(), have: vec![0] };
    let given = a.with(|ctx| ops::keys(ctx, &bid, &ask)).unwrap();
    assert_eq!(given.keys.iter().map(|(e, _)| *e).collect::<Vec<_>>(), vec![1], "only what B lacks");
    assert_eq!(b.with(|ctx| ctx.keys.take(C, &given.keys)), 1);
    assert_eq!(b.open(&after).as_deref(), Some(&b"after the removal"[..]));
    // The removed machine is given nothing, and a key of an epoch the chain has not reached is withheld.
    assert_eq!(a.with(|ctx| ops::keys(ctx, &cid, &KeysReq { collection: C.into(), have: vec![] })), Err(OpError::NotMember));
    assert!(keys::mint(&SqlKeys(&a.conn), C, 2));
    let again = a.with(|ctx| ops::keys(ctx, &bid, &KeysReq { collection: C.into(), have: vec![] })).unwrap();
    assert_eq!(again.keys.iter().map(|(e, _)| *e).collect::<Vec<_>>(), vec![0, 1]);
}

#[test]
fn a_removal_that_loses_a_fork_leaves_no_key_behind_and_one_that_cannot_be_given_a_key_is_not_signed() {
    let (mut a, mut b, mut c) = (n(1), n(2), n(3));
    a.found("kanban", Policy::AutoProven);
    join(&mut a, &mut b, "kanban");
    join(&mut a, &mut c, "kanban");
    catch_up(&mut b, &mut a);
    let policy = a.with(|ctx| ops::change(ctx, C, Change::SetPolicy { policy: Policy::Approve })).unwrap();
    let (gone, cid, aid) = (ended(&[(&a, 0), (&b, 0)]), c.id.clone(), a.id.clone());
    b.with(|ctx| ops::change(ctx, C, Change::Remove { id: cid, ended: gone })).unwrap();
    assert_eq!(b.epochs(), vec![0, 1]);
    // The earlier member's entry wins at B: the removal is gone, and with it the key B made for it.
    let offer = MembersReq { collection: C.into(), have_n: 3, offered: vec![policy] };
    let ans = b.with(|ctx| ops::members(ctx, &aid, &offer)).unwrap();
    assert!(matches!(ans.taken[0], Ok(super::members::Taken::Replaced(_))));
    assert_eq!(b.epochs(), vec![0], "the key of the lost removal is forgotten");

    // A removal that cannot be given a key is not signed.
    let before = a.chains[C].entries.len();
    let mut ctx_cannot = Ctx {
        conn: &a.conn,
        chains: &mut a.chains,
        clock: &mut a.clock,
        pending: &mut a.pending,
        me: Me { id: &a.id, private: &a.seed },
        proof: &Yes,
        keys: &Cannot,
        now_ms: NOW,
        chain_cap: ops::MAX_ENTRIES,
        active: true,
    };
    let cid = c.id.clone();
    let gone = ended(&[(&b, 0)]);
    assert_eq!(ops::change(&mut ctx_cannot, C, Change::Remove { id: cid, ended: gone }), Err(OpError::NoKey));
    assert_eq!(ctx_cannot.chains[C].entries.len(), before);
}

#[test]
fn a_join_waiting_for_a_person_is_still_there_in_the_next_call() {
    let (mut a, b) = (n(1), n(2));
    a.found("kanban", Policy::Approve);
    let req = JoinReq { collection: C.into(), subject: b.subject.clone() };
    let id = b.id.clone();
    assert_eq!(a.with(|ctx| ops::join(ctx, &id, &req)), Ok(Joined::Pending));
    chain_store::save_pending(&a.conn, &a.pending).unwrap();
    a.pending = chain_store::load_pending(&a.conn).unwrap();
    assert_eq!(a.pending.len(), 1);
    let entry: Entry = a.with(|ctx| ops::approve(ctx, C, &id)).unwrap();
    assert_eq!(entry.signer, a.id);
    chain_store::save_pending(&a.conn, &a.pending).unwrap();
    assert!(chain_store::load_pending(&a.conn).unwrap().is_empty());
}
