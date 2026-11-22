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
        let fingerprint = hex(&Sha256::digest(box_key)[..8]);
        Subject { kind: Kind::Recovery, id: format!("recovery-{fingerprint}"), signing_key: None, box_key: hex(box_key), box_sig: None, sealed_label: sealed_label.to_string() }
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
            Kind::Recovery => self.signing_key.is_none() && self.box_sig.is_none(),
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
    /// An entry beside the head (same position, same predecessor) that is not the one with the lower hash.
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
}

impl State {
    fn empty(collection: &str) -> State {
        State { collection: collection.to_string(), members: BTreeMap::new(), epoch: 0, policy: Policy::Approve, n: 0, head: None }
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
            }
            Change::Remove { id, ended } => {
                let removed = self.members.get(id).ok_or(MemberError::BadRemoval)?;
                if removed.kind == Kind::Machine && self.machines() == 1 {
                    return Err(MemberError::BadRemoval);
                }
                if ended.keys().any(|k| k == id || self.members.get(k).map_or(true, |m| m.kind != Kind::Machine)) {
                    return Err(MemberError::BadRemoval);
                }
                self.members.remove(id);
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
    /// It was beside the head with a lower hash, so it replaced the head, which is returned (a fork settled).
    Replaced(Entry),
}

/// The entries a machine holds for one collection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chain {
    pub entries: Vec<Entry>,
}

impl Chain {
    /// The state after the first `upto` entries, or why the chain is not sound.
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
        Ok(Chain { entries })
    }

    pub fn state(&self, collection: &str) -> State {
        Chain::replay(collection, &self.entries).unwrap_or_else(|_| State::empty(collection))
    }

    /// The next entry, signed by `signer` (a machine of this chain) with its key's seed.
    pub fn next(&self, private: &[u8; 32], collection: &str, signer: &str, change: Change) -> Option<Entry> {
        let prev = self.entries.last().map(Entry::hash);
        Entry::sign(private, collection, self.entries.len() as u64 + 1, prev, change, signer)
    }

    /// Takes in an entry from anywhere. It must verify and extend the head; one beside the head (same position and
    /// predecessor) settles by hash, the lower winning on every machine that sees both, so a fork never lasts.
    pub fn take(&mut self, collection: &str, entry: Entry) -> Result<Taken, MemberError> {
        let mut state = Chain::replay(collection, &self.entries)?;
        if self.entries.iter().any(|e| *e == entry) {
            return Ok(Taken::Duplicate);
        }
        match state.apply(&entry) {
            Ok(()) => {
                self.entries.push(entry);
                Ok(Taken::Extended)
            }
            Err(MemberError::DoesNotExtend) if entry.n == self.entries.len() as u64 && entry.n > 0 => {
                let (head, before) = self.entries.split_last().ok_or(MemberError::DoesNotExtend)?;
                let mut base = Chain::replay(collection, before)?;
                base.apply(&entry)?;
                if entry.hash() < head.hash() {
                    let old = self.entries.pop().ok_or(MemberError::DoesNotExtend)?;
                    self.entries.push(entry);
                    Ok(Taken::Replaced(old))
                } else {
                    Err(MemberError::ForkLost)
                }
            }
            Err(e) => Err(e),
        }
    }
}
