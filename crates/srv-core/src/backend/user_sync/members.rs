// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A collection's members (SPEC_USER_DATA_SYNC_LAYER §3.1): one signed, hash-chained log of membership changes.
//! The first entry founds the collection and is signed by the founder; every later entry is signed by a machine that
//! is a member at the point it extends. A client takes an entry only if it verifies against the signer's key as the
//! chain lists it and it extends the chain it holds, so a change that no member signed is refused, wherever it came
//! from.
//!
//! A member is a machine (named by the id its signing key makes, with a box key that key signed, for sealing the
//! collection key to it) or a recovery subject (a box key only: it never signs, never writes a record and is never
//! pinned, but a new epoch's key is sealed to it too). A member's label is inside the seal: this module only carries
//! the sealed bytes.

use std::collections::BTreeMap;

use agentmux_common::jekt_sign::{ed25519_sign_b64, ed25519_verify_b64, wan_instance_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::log::Pins;

const DOMAIN: &str = "agentmux-sync-member-v1";
const BOX_DOMAIN: &str = "agentmux-sync-member-box-v1";
const SEP: char = '\u{1f}';

/// What kind of subject a member is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Machine,
    Recovery,
}

/// How a machine joins: a machine proven by the LAN trust and the account joins at once (`auto_proven`), or a
/// person confirms on an existing member (`approve`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Policy {
    AutoProven,
    Approve,
}

/// A member of a collection. Keys are lower-case hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub kind: Kind,
    /// A machine's origin id (made from its signing key); a recovery subject's fingerprint.
    pub id: String,
    /// A machine's Ed25519 signing key; none for a recovery subject.
    pub signing_key: Option<String>,
    /// The X25519 key the collection key is sealed to.
    pub box_key: String,
    /// A machine's signing key over its box key; none for a recovery subject.
    pub box_sig: Option<String>,
    /// The member's label (a host name), sealed with the collection key: opaque here.
    pub sealed_label: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// A recovery subject's id: made from its box key, so it cannot take another member's id.
fn recovery_id(box_key: &[u8; 32]) -> String {
    format!("recovery-{}", hex(&Sha256::digest(box_key)[..8]))
}

fn box_material(id: &str, box_key: &str) -> String {
    [BOX_DOMAIN, id, box_key].join(&SEP.to_string())
}

impl Subject {
    /// A machine subject for the signing key whose seed is `private`, offering `box_key` (32 bytes) to seal to.
    pub fn machine(private: &[u8; 32], box_key: &[u8; 32], sealed_label: &str) -> Option<Subject> {
        let public = ed25519_dalek::SigningKey::from_bytes(private).verifying_key().to_bytes();
        let id = wan_instance_id(&public);
        let box_key = hex(box_key);
        let box_sig = ed25519_sign_b64(private, &box_material(&id, &box_key))?;
        Some(Subject { kind: Kind::Machine, id, signing_key: Some(hex(&public)), box_key, box_sig: Some(box_sig), sealed_label: sealed_label.to_string() })
    }

    /// A recovery subject: a box key and nothing else.
    pub fn recovery(box_key: &[u8; 32], sealed_label: &str) -> Subject {
        Subject { kind: Kind::Recovery, id: recovery_id(box_key), signing_key: None, box_key: hex(box_key), box_sig: None, sealed_label: sealed_label.to_string() }
    }

    /// A machine's signing key, if this is a machine whose fields are well formed.
    pub fn public(&self) -> Option<[u8; 32]> {
        (self.kind == Kind::Machine).then_some(())?;
        unhex32(self.signing_key.as_deref()?)
    }

    /// Whether the subject is what it says: a machine's id is made from its signing key and its box key is signed by
    /// that key; a recovery subject has neither.
    fn valid(&self) -> bool {
        if unhex32(&self.box_key).is_none() || self.id.is_empty() || self.id.contains(SEP) || self.sealed_label.contains(SEP) {
            return false;
        }
        match self.kind {
            Kind::Machine => match (self.public(), &self.box_sig) {
                (Some(public), Some(sig)) => wan_instance_id(&public) == self.id && ed25519_verify_b64(&public, &box_material(&self.id, &self.box_key), sig),
                _ => false,
            },
            Kind::Recovery => {
                let own = unhex32(&self.box_key).map(|k| recovery_id(&k));
                self.signing_key.is_none() && self.box_sig.is_none() && own.as_deref() == Some(self.id.as_str())
            }
        }
    }

    fn text(&self) -> String {
        let kind = if self.kind == Kind::Machine { "machine" } else { "recovery" };
        [kind, &self.id, self.signing_key.as_deref().unwrap_or("-"), &self.box_key, self.box_sig.as_deref().unwrap_or("-"), &self.sealed_label]
            .join(&SEP.to_string())
    }
}

/// One change to the member list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// The first entry: the founder, and how machines join.
    Found { member: Subject, policy: Policy },
    Add { member: Subject },
    /// A member leaves or is removed: the collection moves to the next key epoch. `ended` is, for each member that
    /// remains, the last sequence it wrote at the epoch being left (the `ended` of `log::set_epoch`).
    Remove { id: String, ended: BTreeMap<String, u64> },
    /// Changes how machines join; signed like any change, so no peer can quietly relax `approve`.
    SetPolicy { policy: Policy },
}

impl Change {
    fn text(&self) -> String {
        let policy = |p: &Policy| if *p == Policy::AutoProven { "auto_proven" } else { "approve" };
        match self {
            Change::Found { member, policy: p } => ["found".to_string(), member.text(), policy(p).to_string()].join(&SEP.to_string()),
            Change::Add { member } => ["add".to_string(), member.text()].join(&SEP.to_string()),
            Change::Remove { id, ended } => {
                let ended: Vec<String> = ended.iter().map(|(k, v)| format!("{k}={v}")).collect();
                ["remove", id, &ended.join(",")].join(&SEP.to_string())
            }
            Change::SetPolicy { policy: p } => ["policy", policy(p)].join(&SEP.to_string()),
        }
    }
}

/// A signed entry of the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub collection: String,
    /// 1 for the founding entry, then each entry one more.
    pub n: u64,
    /// The hash of the entry before it; none for the first.
    pub prev: Option<String>,
    pub change: Change,
    /// The id of the machine that signed.
    pub signer: String,
    pub sig: String,
}

impl Entry {
    fn material(&self) -> String {
        [DOMAIN, &self.collection, &self.n.to_string(), self.prev.as_deref().unwrap_or("-"), &self.change.text(), &self.signer]
            .join(&SEP.to_string())
    }

    /// The hash the next entry names: over everything, signature included.
    pub fn hash(&self) -> String {
        hex(&Sha256::digest(format!("{}{SEP}{}", self.material(), self.sig).as_bytes()))
    }

    /// An entry signed with the seed of the signing machine's key.
    pub fn sign(private: &[u8; 32], collection: &str, n: u64, prev: Option<String>, change: Change, signer: &str) -> Option<Entry> {
        let mut entry = Entry { collection: collection.to_string(), n, prev, change, signer: signer.to_string(), sig: String::new() };
        entry.sig = ed25519_sign_b64(private, &entry.material())?;
        Some(entry)
    }
}

/// Why an entry was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberError {
    /// Not for this collection, or not the next entry, or it does not name the head it extends.
    DoesNotExtend,
    /// The first entry must found the collection, and only the first.
    Founding,
    /// The signer is not a machine that is a member at this point.
    NotAMember,
    /// The signature does not verify against the signer's key as the chain lists it.
    BadSignature,
    /// A subject whose id, keys or signature are not what they say.
    BadSubject,
    AlreadyMember,
    /// A removal of someone who is not a member, or of the last machine, or an `ended` naming a non-member.
    BadRemoval,
    /// An entry beside the head (same position, same predecessor) that settled against it, or a second founding entry.
    ForkLost,
}

/// The members, the epoch and the policy as a chain leaves them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub collection: String,
    pub members: BTreeMap<String, Subject>,
    pub epoch: u32,
    pub policy: Policy,
    pub n: u64,
    pub head: Option<String>,
    /// For each member, the position of the entry that made it one: the founder is 1, so an earlier member ranks
    /// before a later one (this is what settles two members removing each other at once).
    pub joined: BTreeMap<String, u64>,
}

impl State {
    fn empty(collection: &str) -> State {
        State { collection: collection.to_string(), members: BTreeMap::new(), epoch: 0, policy: Policy::Approve, n: 0, head: None, joined: BTreeMap::new() }
    }

    /// The signing keys of the machines, for `log::apply_remote`: a recovery subject is never in it.
    pub fn pins(&self) -> Pins {
        self.members.values().filter_map(|m| Some((m.id.clone(), m.public()?))).collect()
    }

    /// Whether a machine that is a member here is one the caller itself trusts (its own LAN pin and account check):
    /// the anchor of a join. A recovery subject never is.
    pub fn anchored(&self, trusted: &dyn Fn(&str) -> bool) -> bool {
        self.members.values().any(|m| m.kind == Kind::Machine && trusted(&m.id))
    }

    fn machines(&self) -> usize {
        self.members.values().filter(|m| m.kind == Kind::Machine).count()
    }

    /// Checks `entry` against this state and, if it holds, takes it in.
    fn apply(&mut self, entry: &Entry) -> Result<(), MemberError> {
        if entry.collection != self.collection || entry.n != self.n + 1 || entry.prev != self.head {
            return Err(MemberError::DoesNotExtend);
        }
        if (entry.n == 1) != matches!(entry.change, Change::Found { .. }) {
            return Err(MemberError::Founding);
        }
        let signer_key = match &entry.change {
            Change::Found { member, .. } => {
                if !member.valid() || member.kind != Kind::Machine || member.id != entry.signer {
                    return Err(MemberError::BadSubject);
                }
                member.public()
            }
            _ => self.members.get(&entry.signer).filter(|m| m.kind == Kind::Machine).and_then(Subject::public),
        };
        let key = signer_key.ok_or(MemberError::NotAMember)?;
        if !ed25519_verify_b64(&key, &entry.material(), &entry.sig) {
            return Err(MemberError::BadSignature);
        }
        match &entry.change {
            Change::Found { member, policy } => {
                self.members.insert(member.id.clone(), member.clone());
                self.joined.insert(member.id.clone(), entry.n);
                self.policy = *policy;
            }
            Change::Add { member } => {
                if !member.valid() {
                    return Err(MemberError::BadSubject);
                }
                if self.members.contains_key(&member.id) {
                    return Err(MemberError::AlreadyMember);
                }
                self.members.insert(member.id.clone(), member.clone());
                self.joined.insert(member.id.clone(), entry.n);
            }
            Change::Remove { id, ended } => {
                let removed = self.members.get(id).ok_or(MemberError::BadRemoval)?;
                if removed.kind == Kind::Machine && self.machines() == 1 {
                    return Err(MemberError::BadRemoval);
                }
                // `ended` names every machine that remains, and only those: a machine left out would have its
                // old-epoch records refused for good.
                let remaining: Vec<&String> = self.members.values().filter(|m| m.kind == Kind::Machine && m.id != *id).map(|m| &m.id).collect();
                if ended.len() != remaining.len() || remaining.iter().any(|m| !ended.contains_key(*m)) {
                    return Err(MemberError::BadRemoval);
                }
                self.members.remove(id);
                self.joined.remove(id);
                self.epoch += 1;
            }
            Change::SetPolicy { policy } => self.policy = *policy,
        }
        self.n = entry.n;
        self.head = Some(entry.hash());
        Ok(())
    }
}

/// What taking an entry did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Taken {
    /// It extended the chain.
    Extended,
    /// Already held.
    Duplicate,
    /// It was beside the head and settled in its favour: it replaced the head, which is returned.
    Replaced(Entry),
}

/// The entries a machine holds for one collection: the chain, and the other entries seen at the head's position (a
/// fork not yet outgrown), so the winner is a function of everything seen and not of the order it arrived in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chain {
    pub entries: Vec<Entry>,
    contested: Vec<Entry>,
}

/// Which of the entries at one position wins. An entry signed by a member another entry removes loses to it, so a
/// member cannot undo its own removal by racing it; if every entry is removed by another, they all stand. Among
/// those that stand the earliest member wins (a member cannot change when it joined), then the lower hash.
fn settle(base: &State, set: &[Entry]) -> Entry {
    let removed_by_other = |e: &Entry| set.iter().any(|o| o != e && matches!(&o.change, Change::Remove { id, .. } if *id == e.signer));
    let standing: Vec<&Entry> = set.iter().filter(|e| !removed_by_other(e)).collect();
    let pool: Vec<&Entry> = if standing.is_empty() { set.iter().collect() } else { standing };
    let rank = |e: &&Entry| (base.joined.get(&e.signer).copied().unwrap_or(u64::MAX), e.hash());
    pool.into_iter().min_by_key(rank).cloned().unwrap_or_else(|| set[0].clone())
}

impl Chain {
    /// The state after all of `entries`, or why the chain is not sound.
    fn replay(collection: &str, entries: &[Entry]) -> Result<State, MemberError> {
        let mut state = State::empty(collection);
        for e in entries {
            state.apply(e)?;
        }
        Ok(state)
    }

    /// A chain from entries received whole: every one is checked in order.
    pub fn from_entries(collection: &str, entries: Vec<Entry>) -> Result<Chain, MemberError> {
        Chain::replay(collection, &entries)?;
        Ok(Chain { entries, contested: Vec::new() })
    }

    /// The members, epoch and policy the chain leaves, or why the chain is not sound (a chain that was altered
    /// after it was taken in is an error, not an empty collection).
    pub fn state(&self, collection: &str) -> Result<State, MemberError> {
        Chain::replay(collection, &self.entries)
    }

    /// Whether a removal is in doubt: entries beside the head are held that this machine has not outgrown, and one of
    /// them, or the head, removes a member. The key epoch that removal makes is not settled, so nothing is sealed at it.
    pub fn epoch_contested(&self) -> bool {
        let removes = |e: &Entry| matches!(e.change, Change::Remove { .. });
        !self.contested.is_empty() && (self.contested.iter().any(removes) || self.entries.last().is_some_and(removes))
    }

    /// The next entry, signed by `signer` (a machine of this chain) with its key's seed.
    pub fn next(&self, private: &[u8; 32], collection: &str, signer: &str, change: Change) -> Option<Entry> {
        let prev = self.entries.last().map(Entry::hash);
        Entry::sign(private, collection, self.entries.len() as u64 + 1, prev, change, signer)
    }

    /// Takes in an entry from anywhere. It must verify and extend the head. An entry beside the head (same position
    /// and predecessor, valid there) is a fork: the winner is [`settle`]d over every entry seen at that position, the
    /// same on every machine that sees them all. The founding entry is never replaced: a second one is a different
    /// collection, and who founded this one is anchored by the joiner's own trust (`State::anchored`).
    pub fn take(&mut self, collection: &str, entry: Entry) -> Result<Taken, MemberError> {
        if self.entries.contains(&entry) || self.contested.contains(&entry) {
            return Ok(Taken::Duplicate);
        }
        let mut state = Chain::replay(collection, &self.entries)?;
        match state.apply(&entry) {
            Ok(()) => {
                self.entries.push(entry);
                self.contested.clear();
                Ok(Taken::Extended)
            }
            Err(MemberError::DoesNotExtend) if entry.n == self.entries.len() as u64 && entry.n > 1 => self.fork(collection, entry),
            Err(MemberError::DoesNotExtend) if entry.n == 1 && !self.entries.is_empty() => Err(MemberError::ForkLost),
            Err(e) => Err(e),
        }
    }

    fn fork(&mut self, collection: &str, entry: Entry) -> Result<Taken, MemberError> {
        let (head, before) = self.entries.split_last().ok_or(MemberError::DoesNotExtend)?;
        let mut base = Chain::replay(collection, before)?;
        let at = base.clone();
        base.apply(&entry)?;
        let mut set = self.contested.clone();
        set.push(head.clone());
        set.push(entry.clone());
        let winner = settle(&at, &set);
        let old = head.clone();
        let won = winner != old;
        set.retain(|e| *e != winner);
        set.sort_by_key(Entry::hash);
        self.contested = set;
        if won {
            self.entries.pop();
            self.entries.push(winner);
            Ok(Taken::Replaced(old))
        } else {
            Err(MemberError::ForkLost)
        }
    }
}
