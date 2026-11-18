// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The names Windows gives the instances of its GPU performance counters, parsed. They carry the
//! process, the adapter and the engine, which is all the per-process and per-adapter figures are made from
//! (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md §3):
//!
//! ```text
//! \GPU Engine(pid_17408_luid_0x00000000_0x0000F3A8_phys_0_eng_0_engtype_3D)\Utilization Percentage
//! \GPU Adapter Memory(luid_0x00000000_0x0000F3A8_phys_0)\Dedicated Usage
//! \GPU Process Memory(pid_17408_luid_0x00000000_0x0000F3A8_phys_0)\Dedicated Usage
//! ```
//!
//! Pure text, so the tests run on every platform.

/// One engine of one adapter, as one process uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInstance {
    pub pid: u32,
    /// The adapter ([`Luid`]).
    pub adapter: String,
    /// The engine on that adapter, `<index>:<type>` (`0:3d`): one engine is what a figure of "how busy" is of.
    pub engine: String,
}

/// An adapter's locally unique id in the one spelling both the counters and DXGI are put in, `0x00000000_0x0000F3A8`.
pub fn luid(high: i32, low: u32) -> String {
    format!("0x{:08X}_0x{:08X}", high as u32, low)
}

/// `0x0000f3a8` as a number; the counters write the parts in hex with a prefix.
fn hex(token: &str) -> Option<u32> {
    u32::from_str_radix(token.strip_prefix("0x")?, 16).ok()
}

/// The tokens of an instance name, lower case: a name is words and numbers joined by underscores.
fn tokens(name: &str) -> Vec<String> {
    name.to_ascii_lowercase().split('_').map(str::to_string).collect()
}

/// What follows `key` in the tokens, as a number.
fn number_after(tokens: &[String], key: &str) -> Option<u32> {
    let at = tokens.iter().position(|t| t == key)?;
    tokens.get(at + 1)?.parse().ok()
}

/// The adapter named after `luid` in the tokens.
fn adapter_in(tokens: &[String]) -> Option<String> {
    let at = tokens.iter().position(|t| t == "luid")?;
    let high = hex(tokens.get(at + 1)?)?;
    let low = hex(tokens.get(at + 2)?)?;
    Some(luid(high as i32, low))
}

/// `pid_<n>_luid_<high>_<low>_phys_<k>_eng_<i>_engtype_<type>`: a `GPU Engine` instance.
pub fn parse_engine(name: &str) -> Option<EngineInstance> {
    let t = tokens(name);
    let pid = number_after(&t, "pid")?;
    let adapter = adapter_in(&t)?;
    let index = number_after(&t, "eng")?;
    let at = t.iter().position(|x| x == "engtype")?;
    // A type is a word ("3d", "copy", "videodecode"), but join what follows in case one ever has an underscore.
    let kind = t.get(at + 1..)?.join("_");
    if kind.is_empty() {
        return None;
    }
    Some(EngineInstance { pid, adapter, engine: format!("{index}:{kind}") })
}

/// `luid_<high>_<low>_phys_<k>`: a `GPU Adapter Memory` instance.
pub fn parse_adapter(name: &str) -> Option<String> {
    adapter_in(&tokens(name))
}

/// `pid_<n>_luid_<high>_<low>_phys_<k>`: a `GPU Process Memory` instance, the process and the adapter its memory is on.
pub fn parse_process_memory(name: &str) -> Option<(u32, String)> {
    let t = tokens(name);
    Some((number_after(&t, "pid")?, adapter_in(&t)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADAPTER: &str = "0x00000000_0x0000F3A8";

    #[test]
    fn an_engine_instance_names_process_adapter_and_engine() {
        let e = parse_engine("pid_17408_luid_0x00000000_0x0000F3A8_phys_0_eng_3_engtype_3D").unwrap();
        assert_eq!(e, EngineInstance { pid: 17408, adapter: ADAPTER.into(), engine: "3:3d".into() });
    }

    #[test]
    fn the_case_and_the_engine_type_spelling_do_not_matter() {
        let e = parse_engine("PID_4_LUID_0X00000000_0X0000f3a8_PHYS_0_ENG_12_ENGTYPE_VideoDecode").unwrap();
        assert_eq!((e.pid, e.adapter.as_str(), e.engine.as_str()), (4, ADAPTER, "12:videodecode"));
    }

    #[test]
    fn a_high_part_that_is_negative_in_dxgi_matches_its_hex_form() {
        assert_eq!(luid(-1, 0x1A), "0xFFFFFFFF_0x0000001A");
        let e = parse_engine("pid_1_luid_0xFFFFFFFF_0x0000001A_phys_0_eng_0_engtype_Copy").unwrap();
        assert_eq!(e.adapter, luid(-1, 0x1A));
    }

    #[test]
    fn an_adapter_memory_instance_names_the_adapter() {
        assert_eq!(parse_adapter("luid_0x00000000_0x0000F3A8_phys_0").as_deref(), Some(ADAPTER));
    }

    #[test]
    fn a_process_memory_instance_names_process_and_adapter() {
        assert_eq!(
            parse_process_memory("pid_980_luid_0x00000000_0x0000F3A8_phys_0"),
            Some((980, ADAPTER.to_string()))
        );
    }

    #[test]
    fn a_name_that_is_not_one_of_these_is_not_parsed() {
        assert_eq!(parse_engine("_Total"), None);
        assert_eq!(parse_engine("pid_x_luid_0x0_0x1_phys_0_eng_0_engtype_3D"), None);
        assert_eq!(parse_engine("pid_1_luid_0x00000000_0x0000F3A8_phys_0_eng_0"), None);
        assert_eq!(parse_adapter("pid_1"), None);
        assert_eq!(parse_process_memory("luid_0x00000000_0x0000F3A8_phys_0"), None);
    }
}
