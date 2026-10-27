// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The machine's network byte counters for the telemetry tick (`sysinfo.rs`).
//!
//! The tick only needs the bytes sent and received, summed over the machine's real interfaces.
//! `sysinfo::Networks::refresh` on Windows also looks up every adapter's MAC and IP addresses on
//! each call (`GetAdaptersAddresses`), which srv never reads: about a tenth of an idle srv's CPU.
//! On Windows this reads the counters from `GetIfTable2` directly, with the interface filter
//! sysinfo applies; elsewhere it still uses sysinfo.

/// One row of the interface table, reduced to what the filter and the sum need.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IfRow {
    /// The interface has a link speed, is connected and has a hardware address.
    pub usable: bool,
    /// The interface's GUID without its first field, which is how sysinfo groups software
    /// interfaces (a virtual switch and its members) with the hardware one.
    pub group: [u8; 12],
    pub in_octets: u64,
    pub out_octets: u64,
}

/// `(bytes sent, bytes received)` over the usable interfaces that are alone in their group,
/// which is the set `sysinfo::Networks` keeps.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn sum_rows(rows: &[IfRow]) -> (u64, u64) {
    let mut groups = std::collections::HashMap::new();
    for r in rows.iter().filter(|r| r.usable) {
        *groups.entry(r.group).or_insert(0u32) += 1;
    }
    rows.iter()
        .filter(|r| r.usable && groups[&r.group] == 1)
        .fold((0, 0), |(sent, recv), r| (sent.saturating_add(r.out_octets), recv.saturating_add(r.in_octets)))
}

#[cfg(windows)]
mod imp {
    use super::{sum_rows, IfRow};
    use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows_sys::Win32::NetworkManagement::Ndis::MediaConnectStateDisconnected;

    pub struct NetCounters;

    impl NetCounters {
        pub fn new() -> Self {
            NetCounters
        }

        /// `(bytes sent, bytes received)`, or zeros when the table can't be read.
        pub fn totals(&mut self) -> (u64, u64) {
            read_rows().map_or((0, 0), |rows| sum_rows(&rows))
        }
    }

    fn read_rows() -> Option<Vec<IfRow>> {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        // SAFETY: on success `table` points at a table that lives until `FreeMibTable`; its
        // `NumEntries` rows are read before that and copied out.
        unsafe {
            if GetIfTable2(&mut table) != 0 || table.is_null() {
                return None;
            }
            let first = (*table).Table.as_ptr();
            let rows = (0..(*table).NumEntries as usize)
                .map(|i| {
                    let r = &*first.add(i);
                    let g = r.InterfaceGuid;
                    let mut group = [0u8; 12];
                    group[..2].copy_from_slice(&g.data2.to_le_bytes());
                    group[2..4].copy_from_slice(&g.data3.to_le_bytes());
                    group[4..].copy_from_slice(&g.data4);
                    IfRow {
                        usable: !(r.TransmitLinkSpeed == 0 && r.ReceiveLinkSpeed == 0)
                            && r.MediaConnectState != MediaConnectStateDisconnected
                            && r.PhysicalAddressLength != 0,
                        group,
                        in_octets: r.InOctets,
                        out_octets: r.OutOctets,
                    }
                })
                .collect();
            FreeMibTable(table as _);
            Some(rows)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct NetCounters(sysinfo::Networks);

    impl NetCounters {
        pub fn new() -> Self {
            NetCounters(sysinfo::Networks::new_with_refreshed_list())
        }

        /// `(bytes sent, bytes received)` since each interface appeared.
        pub fn totals(&mut self) -> (u64, u64) {
            self.0.refresh(true);
            self.0.iter().fold((0, 0), |(sent, recv), (_, d)| (sent + d.total_transmitted(), recv + d.total_received()))
        }
    }
}

pub use imp::NetCounters;

#[cfg(test)]
mod tests {
    use super::*;

    fn row(group: u8, usable: bool, inb: u64, out: u64) -> IfRow {
        IfRow { usable, group: [group; 12], in_octets: inb, out_octets: out }
    }

    #[test]
    fn sums_the_usable_interfaces() {
        let rows = [row(1, true, 100, 10), row(2, true, 200, 20)];
        assert_eq!(sum_rows(&rows), (30, 300));
    }

    #[test]
    fn leaves_out_unusable_interfaces() {
        let rows = [row(1, true, 100, 10), row(2, false, 5_000, 500)];
        assert_eq!(sum_rows(&rows), (10, 100));
    }

    #[test]
    fn leaves_out_software_interfaces_that_share_a_hardware_guid() {
        // two usable rows in one group (a virtual switch and its member) are both dropped, as sysinfo drops them
        let rows = [row(1, true, 100, 10), row(1, true, 900, 90), row(2, true, 200, 20)];
        assert_eq!(sum_rows(&rows), (20, 200));
    }

    #[test]
    fn an_unusable_row_does_not_make_its_group_shared() {
        let rows = [row(1, true, 100, 10), row(1, false, 900, 90)];
        assert_eq!(sum_rows(&rows), (10, 100));
    }

    #[test]
    fn no_interfaces_sum_to_zero() {
        assert_eq!(sum_rows(&[]), (0, 0));
    }

    #[test]
    fn the_live_table_is_readable_and_counts_never_go_backwards() {
        let mut counters = NetCounters::new();
        let (sent_a, recv_a) = counters.totals();
        let (sent_b, recv_b) = counters.totals();
        assert!(sent_b >= sent_a && recv_b >= recv_a, "{sent_a}/{recv_a} then {sent_b}/{recv_b}");
    }
}
