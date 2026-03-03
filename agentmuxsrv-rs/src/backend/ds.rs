// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Concurrent data structures.
//! Port of Go's `pkg/util/ds/` (expmap.go, syncmap.go).
//!
//! - `SyncMap<T>`: Thread-safe generic map with test-and-set.
//! - `ExpMap<T>`: Thread-safe map with auto-expiring entries.

#![allow(dead_code)]

use std::collections::{BinaryHeap, HashMap};
use std::sync::Mutex;
use std::time::Instant;

// ---- SyncMap ----

/// A thread-safe generic map backed by a Mutex.
pub struct SyncMap<T> {
    inner: Mutex<HashMap<String, T>>,
}

impl<T: Clone> Default for SyncMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> SyncMap<T> {
    /// Create a new empty SyncMap.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Insert or update a value.
    pub fn set(&self, key: &str, value: T) {
        self.inner.lock().unwrap().insert(key.to_string(), value);
    }

    /// Get a value by key. Returns the default if not found.
    pub fn get(&self, key: &str) -> Option<T> {
        self.inner.lock().unwrap().get(key).cloned()
    }

    /// Get a value with existence check.
    pub fn get_ex(&self, key: &str) -> (Option<T>, bool) {
        let map = self.inner.lock().unwrap();
        match map.get(key) {
            Some(v) => (Some(v.clone()), true),
            None => (None, false),
        }
    }

    /// Remove a key.
    pub fn delete(&self, key: &str) {
        self.inner.lock().unwrap().remove(key);
    }

    /// Set only if the key does not already exist.
    /// Returns true if the value was set.
    pub fn set_unless(&self, key: &str, value: T) -> bool {
        let mut map = self.inner.lock().unwrap();
        if map.contains_key(key) {
            return false;
        }
        map.insert(key.to_string(), value);
        true
    }

    /// Atomically test the current value and set a new one if the test passes.
    /// `test_fn` receives `(current_value, exists)` and returns true to proceed.
    pub fn test_and_set<F>(&self, key: &str, new_value: T, test_fn: F) -> bool
    where
        F: FnOnce(Option<&T>, bool) -> bool,
    {
        let mut map = self.inner.lock().unwrap();
        let (current, exists) = match map.get(key) {
            Some(v) => (Some(v), true),
            None => (None, false),
        };
        if test_fn(current, exists) {
            map.insert(key.to_string(), new_value);
            true
        } else {
            false
        }
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}

// ---- ExpMap ----

/// Entry in the expiration heap.
#[derive(Eq, PartialEq)]
struct ExpEntry {
    key: String,
    exp: Instant,
}

impl Ord for ExpEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse ordering for min-heap (earliest expiry first)
        other.exp.cmp(&self.exp)
    }
}

impl PartialOrd for ExpEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Value entry with expiration time.
struct ExpMapEntry<T> {
    val: T,
    exp: Instant,
}

/// A thread-safe map with auto-expiring entries.
///
/// Expired entries are lazily garbage-collected on `get()`.
pub struct ExpMap<T> {
    inner: Mutex<ExpMapInner<T>>,
}

struct ExpMapInner<T> {
    map: HashMap<String, ExpMapEntry<T>>,
    heap: BinaryHeap<ExpEntry>,
}

impl<T: Clone> Default for ExpMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> ExpMap<T> {
    /// Create a new empty ExpMap.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ExpMapInner {
                map: HashMap::new(),
                heap: BinaryHeap::new(),
            }),
        }
    }

    /// Insert or update a value with an expiration time.
    pub fn set(&self, key: &str, value: T, exp: Instant) {
        let mut inner = self.inner.lock().unwrap();
        let old_exp = inner.map.get(key).map(|e| e.exp);
        inner.map.insert(
            key.to_string(),
            ExpMapEntry {
                val: value,
                exp,
            },
        );
        // Only push to heap if expiration changed
        if old_exp != Some(exp) {
            inner.heap.push(ExpEntry {
                key: key.to_string(),
                exp,
            });
        }
    }

    /// Get a value, triggering lazy expiration of old entries.
    pub fn get(&self, key: &str) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        expire_items(&mut inner);
        inner.map.get(key).map(|e| e.val.clone())
    }

    /// Get the number of non-expired entries.
    pub fn len(&self) -> usize {
        let mut inner = self.inner.lock().unwrap();
        expire_items(&mut inner);
        inner.map.len()
    }

    /// Check if empty (after expiration).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Remove expired entries from the map.
fn expire_items<T>(inner: &mut ExpMapInner<T>) {
    let now = Instant::now();
    while let Some(top) = inner.heap.peek() {
        if top.exp > now {
            break;
        }
        let entry = inner.heap.pop().unwrap();
        // Only remove from map if the expiration still matches
        if let Some(map_entry) = inner.map.get(&entry.key) {
            if map_entry.exp <= now {
                inner.map.remove(&entry.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ---- SyncMap Tests ----

    #[test]
    fn test_syncmap_set_get() {
        let m = SyncMap::new();
        m.set("key", 42);
        assert_eq!(m.get("key"), Some(42));
    }

    #[test]
    fn test_syncmap_get_missing() {
        let m: SyncMap<i32> = SyncMap::new();
        assert_eq!(m.get("missing"), None);
    }

    #[test]
    fn test_syncmap_get_ex() {
        let m = SyncMap::new();
        m.set("key", "value".to_string());
        let (val, exists) = m.get_ex("key");
        assert!(exists);
        assert_eq!(val.unwrap(), "value");

        let (val, exists) = m.get_ex("missing");
        assert!(!exists);
        assert!(val.is_none());
    }

    #[test]
    fn test_syncmap_delete() {
        let m = SyncMap::new();
        m.set("key", 1);
        m.delete("key");
        assert_eq!(m.get("key"), None);
    }

    #[test]
    fn test_syncmap_set_unless() {
        let m = SyncMap::new();
        assert!(m.set_unless("key", 1));
        assert!(!m.set_unless("key", 2)); // Already exists
        assert_eq!(m.get("key"), Some(1)); // Original value preserved
    }

    #[test]
    fn test_syncmap_test_and_set() {
        let m = SyncMap::new();
        m.set("key", 10);

        // Test passes: current is 10
        let result = m.test_and_set("key", 20, |current, exists| {
            exists && current == Some(&10)
        });
        assert!(result);
        assert_eq!(m.get("key"), Some(20));

        // Test fails: current is 20, not 10
        let result = m.test_and_set("key", 30, |current, exists| {
            exists && current == Some(&10)
        });
        assert!(!result);
        assert_eq!(m.get("key"), Some(20)); // Unchanged
    }

    #[test]
    fn test_syncmap_len() {
        let m = SyncMap::new();
        assert!(m.is_empty());
        m.set("a", 1);
        m.set("b", 2);
        assert_eq!(m.len(), 2);
    }

    // ---- ExpMap Tests ----

    #[test]
    fn test_expmap_set_get() {
        let m = ExpMap::new();
        let exp = Instant::now() + Duration::from_secs(60);
        m.set("key", 42, exp);
        assert_eq!(m.get("key"), Some(42));
    }

    #[test]
    fn test_expmap_get_missing() {
        let m: ExpMap<i32> = ExpMap::new();
        assert_eq!(m.get("missing"), None);
    }

    #[test]
    fn test_expmap_expired_entry() {
        let m = ExpMap::new();
        // Set with already-expired time
        let exp = Instant::now() - Duration::from_secs(1);
        m.set("key", 42, exp);
        // Should not be found (expired)
        assert_eq!(m.get("key"), None);
    }

    #[test]
    fn test_expmap_update_expiry() {
        let m = ExpMap::new();
        let short = Instant::now() - Duration::from_secs(1);
        let long = Instant::now() + Duration::from_secs(60);

        m.set("key", 1, short);
        assert_eq!(m.get("key"), None); // Expired

        m.set("key", 2, long);
        assert_eq!(m.get("key"), Some(2)); // Renewed
    }

    #[test]
    fn test_expmap_multiple_entries() {
        let m = ExpMap::new();
        let future = Instant::now() + Duration::from_secs(60);
        let past = Instant::now() - Duration::from_secs(1);

        m.set("alive", 1, future);
        m.set("dead", 2, past);

        assert_eq!(m.get("alive"), Some(1));
        assert_eq!(m.get("dead"), None);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn test_expmap_len_after_expiry() {
        let m = ExpMap::new();
        let past = Instant::now() - Duration::from_secs(1);
        m.set("a", 1, past);
        m.set("b", 2, past);
        assert_eq!(m.len(), 0);
        assert!(m.is_empty());
    }
}
