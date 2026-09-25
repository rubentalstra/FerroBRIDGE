// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The in-process claim a delivery holds on its source keys while it commits.
//!
//! The identity lookup and the commit are separate steps, so two deliveries of
//! one source that arrive at once could both find it unrecorded and both
//! commit. A delivery claims every source key it carries before it reads the
//! identity map and releases them when it finishes, whichever way it
//! finishes. A delivery that finds a key claimed commits nothing and is
//! answered `409`, and the caller retries once the first delivery settles.
//! No specification governs the redelivery rule: our own design.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

/// The source keys every in-flight delivery of one deployment holds.
///
/// One value serves one identity store, because a key names a source only
/// within the store that records it.
#[derive(Debug, Default)]
pub struct Claims {
    /// The storage keys claimed now, as
    /// [`SourceVersion::storage_key`](crate::facade::identity::record::SourceVersion::storage_key)
    /// spells them.
    held: Mutex<BTreeSet<String>>,
}

/// Every key of a refused claim that another in-flight delivery holds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("another delivery holds {} of these source keys", .held.len())]
pub struct Contended {
    /// The keys another delivery holds.
    held: BTreeSet<String>,
}

impl Contended {
    /// Returns whether another delivery holds `key`.
    #[must_use]
    pub fn holds(&self, key: &str) -> bool {
        self.held.contains(key)
    }
}

/// The keys one delivery holds, released when the value drops.
///
/// Dropping runs on every exit of the delivery, a returned refusal and an
/// unwinding panic alike, so no path leaves a key claimed.
#[derive(Debug)]
#[must_use = "a claim releases its keys the moment it drops"]
pub struct Claim<'c> {
    /// The set the keys are released into.
    claims: &'c Claims,
    /// The keys this delivery holds.
    keys: BTreeSet<String>,
}

impl Claims {
    /// Returns a set with no key claimed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: Mutex::new(BTreeSet::new()),
        }
    }

    /// Claims every key of `keys` for one delivery, or none of them.
    ///
    /// An empty `keys` always succeeds: a delivery with no source key cannot
    /// be recognised, so there is nothing to hold.
    ///
    /// # Errors
    ///
    /// Returns [`Contended`] naming every key another in-flight delivery
    /// holds; no key of `keys` is claimed then.
    pub fn claim(&self, keys: impl IntoIterator<Item = String>) -> Result<Claim<'_>, Contended> {
        let keys: BTreeSet<String> = keys.into_iter().collect();
        let mut held = self.lock();
        let contended: BTreeSet<String> = keys.intersection(&held).cloned().collect();
        if !contended.is_empty() {
            return Err(Contended { held: contended });
        }
        held.extend(keys.iter().cloned());
        drop(held);
        Ok(Claim { claims: self, keys })
    }

    /// Returns whether a delivery holds `key` now.
    #[must_use]
    pub fn is_claimed(&self, key: &str) -> bool {
        self.lock().contains(key)
    }

    /// Locks the set.
    fn lock(&self) -> MutexGuard<'_, BTreeSet<String>> {
        // NOTE: no specification governs this: our own design; the lock covers
        // only an insert or a remove, so a poisoned set is still consistent.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Claim<'_> {
    /// Returns whether this claim holds `key`.
    #[must_use]
    pub fn holds(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut held = self.claims.lock();
        for key in &self.keys {
            held.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Claims;

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| String::from(*name)).collect()
    }

    #[test]
    fn a_claim_releases_its_keys_on_drop() {
        let claims = Claims::new();
        let claim = claims
            .claim(keys(&["a", "b"]))
            .expect("nothing is held yet");
        assert!(claims.is_claimed("a") && claims.is_claimed("b"));
        drop(claim);
        assert!(!claims.is_claimed("a") && !claims.is_claimed("b"));
        let again = claims.claim(keys(&["a", "b"]));
        assert!(again.is_ok(), "a released key can be claimed again");
    }

    #[test]
    fn a_contended_claim_names_the_held_keys_and_holds_none() {
        let claims = Claims::new();
        let first = claims.claim(keys(&["a"])).expect("nothing is held yet");
        let refused = claims
            .claim(keys(&["a", "b"]))
            .expect_err("another delivery holds a");
        assert!(refused.holds("a"));
        assert!(!refused.holds("b"));
        assert!(
            !claims.is_claimed("b"),
            "a refused claim takes none of its free keys"
        );
        assert!(first.holds("a"));
    }

    #[test]
    fn a_claim_releases_its_keys_when_its_holder_panics() {
        let claims = Claims::new();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = claims.claim(keys(&["a"])).expect("nothing is held yet");
            panic!("the delivery panics while it holds the claim");
        }));
        assert!(unwound.is_err());
        assert!(!claims.is_claimed("a"), "unwinding released the key");
    }

    #[test]
    fn an_empty_claim_always_succeeds() {
        let claims = Claims::new();
        let first = claims.claim(Vec::new()).expect("nothing to hold");
        let second = claims.claim(Vec::new()).expect("nothing to hold");
        drop((first, second));
    }
}
