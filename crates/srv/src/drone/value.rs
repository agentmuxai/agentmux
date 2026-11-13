// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A value in a run, with what the engine must remember about where it came
//! from (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §4.2, §4.5).
//!
//! Two bits travel with every value:
//!
//! * `tainted`: some text in it came from outside the person's own design (an
//!   event payload, an API response, a Run step's output, an Agent's answer). It
//!   may be quoted, never used as a command, a path or a target.
//! * `derived`: a clean number or boolean made *inside the run* from tainted
//!   input (a `count` of a tainted list, an exit code, an enum cleared by
//!   `in_set`). It is clean as data, and still **chosen by an attacker** as
//!   control (a branch, a loop guard) and as a numeric argument. Only a field
//!   typed by the trigger's own schema is exempt (§4.5, the derived-value rule).
//!
//! Every module of the engine takes and returns [`Tv`], so a taint bit is never
//! dropped at a module boundary. Both bits are always written and always
//! required when read: a value with a bit missing is an error, never "clean".

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tv {
    pub v: Value,
    pub tainted: bool,
    pub derived: bool,
}

impl Tv {
    /// A value the person's own design wrote, or a field typed by the trigger's schema.
    pub fn clean(v: Value) -> Self {
        Self { v, tainted: false, derived: false }
    }

    /// A value that carries outside text.
    pub fn tainted(v: Value) -> Self {
        Self { v, tainted: true, derived: false }
    }

    /// A clean number or boolean made in the run from tainted input.
    pub fn derived(v: Value) -> Self {
        Self { v, tainted: false, derived: true }
    }

    /// Whether an attacker could have chosen this value when it is used as a
    /// branch, a loop guard, an item count or a numeric argument (§4.5).
    pub fn attacker_chosen_as_control(&self) -> bool {
        self.tainted || self.derived
    }

    /// May this value be an argument of a command, a path or a target? Only a
    /// value that is neither tainted nor derived (§3.6, §4.1).
    pub fn usable_as_argument(&self) -> bool {
        !self.tainted && !self.derived
    }
}

/// The bits of a value made from `inputs`: tainted if any input is, derived if
/// any input is derived and the result is not tainted. This is the default for
/// anything computed from several values; an operation that cannot carry text
/// may lower `tainted` to `derived` (see `transform::taint_out`).
pub fn join(inputs: &[&Tv]) -> (bool, bool) {
    let tainted = inputs.iter().any(|t| t.tainted);
    let derived = !tainted && inputs.iter().any(|t| t.derived);
    (tainted, derived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_value_says_whether_an_attacker_could_have_chosen_it() {
        assert!(!Tv::clean(json!(3)).attacker_chosen_as_control());
        assert!(Tv::tainted(json!("x")).attacker_chosen_as_control());
        assert!(Tv::derived(json!(3)).attacker_chosen_as_control(), "a count of a tainted list gates nothing alone");
    }

    #[test]
    fn only_a_clean_value_is_an_argument() {
        assert!(Tv::clean(json!("v1.2.3")).usable_as_argument());
        assert!(!Tv::tainted(json!("v1.2.3")).usable_as_argument());
        assert!(!Tv::derived(json!(3)).usable_as_argument());
    }

    #[test]
    fn joining_taints_if_any_input_is_and_derives_otherwise() {
        let (c, t, d) = (Tv::clean(json!(1)), Tv::tainted(json!("x")), Tv::derived(json!(2)));
        assert_eq!(join(&[&c, &c]), (false, false));
        assert_eq!(join(&[&c, &t]), (true, false));
        assert_eq!(join(&[&c, &d]), (false, true));
        assert_eq!(join(&[&t, &d]), (true, false), "tainted wins: it is the stronger claim");
        assert_eq!(join(&[]), (false, false));
    }

    #[test]
    fn the_bits_survive_serialization_and_a_missing_bit_is_an_error_not_clean() {
        let t = Tv::tainted(json!({ "a": 1 }));
        assert_eq!(serde_json::from_value::<Tv>(serde_json::to_value(&t).unwrap()).unwrap(), t);
        let d = Tv::derived(json!(2));
        assert_eq!(serde_json::from_value::<Tv>(serde_json::to_value(&d).unwrap()).unwrap(), d);
        assert_eq!(serde_json::to_value(Tv::clean(json!(1))).unwrap(), json!({ "v": 1, "tainted": false, "derived": false }));
        // A value with a bit missing is refused: restoring it as clean would launder taint.
        assert!(serde_json::from_value::<Tv>(json!({ "v": 1 })).is_err());
        assert!(serde_json::from_value::<Tv>(json!({ "v": 1, "tainted": false })).is_err());
    }
}
