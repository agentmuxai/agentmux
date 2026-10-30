// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

const GIB: u64 = 1024 * 1024 * 1024;

fn daemon(gib: u64, ncpu: u32) -> Option<DaemonCapacity> {
    Some(DaemonCapacity { mem_total: gib * GIB, ncpu })
}

#[test]
fn the_default_memory_is_half_of_the_daemon_with_a_floor() {
    // (daemon GiB, default GiB): the 2 GiB floor, then half.
    for (total, want) in [(2, 2), (3, 2), (4, 2), (8, 4), (16, 8), (64, 32)] {
        assert_eq!(auto_memory_bytes(total * GIB, None), want * GIB, "a {total} GiB daemon");
    }
    assert_eq!(auto_memory_bytes(5 * GIB, None), 5 * GIB / 2, "half, between the floor and the total");
}

#[test]
fn an_images_recommended_minimum_raises_the_default_but_not_past_the_daemon() {
    assert_eq!(auto_memory_bytes(16 * GIB, Some(12 * GIB)), 12 * GIB);
    assert_eq!(auto_memory_bytes(16 * GIB, Some(4 * GIB)), 8 * GIB, "a lower minimum changes nothing");
    assert_eq!(auto_memory_bytes(8 * GIB, Some(12 * GIB)), 8 * GIB, "bounded by the daemon");
}

#[test]
fn the_default_cpus_are_all_but_a_quarter_with_one_always_left() {
    for (ncpu, want) in [(1, 1), (2, 1), (4, 3), (8, 6), (16, 12), (64, 48)] {
        assert_eq!(auto_cpu_millis(ncpu), want * 1000, "{ncpu} cores");
    }
}

#[test]
fn a_stored_value_reads_as_auto_none_or_a_number() {
    let v = |j: serde_json::Value| Limit::from_json(Some(&j), 1);
    assert_eq!(Limit::from_json(None, 1), Limit::Auto);
    assert_eq!(v(json!("auto")), Limit::Auto);
    assert_eq!(v(json!("")), Limit::Auto);
    assert_eq!(v(json!(0)), Limit::Unlimited);
    assert_eq!(v(json!("none")), Limit::Unlimited);
    assert_eq!(v(json!(4096)), Limit::Fixed(4096));
    assert_eq!(v(json!("4096")), Limit::Fixed(4096));
    assert_eq!(v(json!(-5)), Limit::Auto, "a negative number is no setting");
    assert_eq!(v(json!("lots")), Limit::Auto);
    assert_eq!(Limit::from_json(Some(&json!(1.5)), 1000), Limit::Fixed(1500), "CPUs are kept in thousandths");
}

#[test]
fn the_agents_setting_wins_over_the_apps_which_wins_over_the_default() {
    let auto = ContainerLimits::default();
    let app = ContainerLimits { memory: Limit::Fixed(6144), cpus: Limit::Fixed(2000) };
    let own = ContainerLimits { memory: Limit::Fixed(1024), cpus: Limit::Unlimited };
    let cap = daemon(16, 8);
    assert_eq!(Wanted::resolve(auto, auto, cap, None), Wanted { memory: Target::Set(8 * GIB as i64), cpus: Target::Set(6_000_000_000) });
    assert_eq!(Wanted::resolve(auto, app, cap, None), Wanted { memory: Target::Set(6144 * 1024 * 1024), cpus: Target::Set(2_000_000_000) });
    // No limit is everything the daemon has: `docker update` cannot clear a limit, only move it.
    assert_eq!(Wanted::resolve(own, app, cap, None), Wanted { memory: Target::Set(1024 * 1024 * 1024), cpus: Target::Set(8_000_000_000) });
}

#[test]
fn a_daemon_that_cannot_be_read_leaves_the_auto_limits_alone_but_keeps_explicit_ones() {
    let wanted = Wanted::resolve(ContainerLimits { memory: Limit::Fixed(2048), cpus: Limit::Auto }, ContainerLimits::default(), None, None);
    assert_eq!(wanted, Wanted { memory: Target::Set(2048 * 1024 * 1024), cpus: Target::Leave });
}

#[test]
fn a_new_container_gets_memory_and_swap_equal_so_it_cannot_page_the_host() {
    let mut host_config = HostConfig::default();
    Wanted { memory: Target::Set(8 * GIB as i64), cpus: Target::Set(6_000_000_000) }.apply_to(&mut host_config);
    assert_eq!((host_config.memory, host_config.memory_swap), (Some(8 * GIB as i64), Some(8 * GIB as i64)));
    assert_eq!((host_config.nano_cpus, host_config.pids_limit), (Some(6_000_000_000), Some(8192)));

    let mut free = HostConfig::default();
    Wanted { memory: Target::Leave, cpus: Target::Leave }.apply_to(&mut free);
    assert_eq!((free.memory, free.memory_swap, free.nano_cpus), (None, None, None), "a limit that is not known is left out, not sent as zero");
}

fn host_config(memory: i64, swap: i64, nano_cpus: i64, pids: i64) -> HostConfig {
    HostConfig { memory: Some(memory), memory_swap: Some(swap), nano_cpus: Some(nano_cpus), pids_limit: Some(pids), ..Default::default() }
}

#[test]
fn a_container_that_already_matches_is_not_updated() {
    let wanted = Wanted { memory: Target::Set(8 * GIB as i64), cpus: Target::Set(6_000_000_000) };
    assert!(wanted.update_from(Some(&host_config(8 * GIB as i64, 8 * GIB as i64, 6_000_000_000, 8192))).is_none());
}

#[test]
fn a_container_made_before_limits_existed_gets_all_of_them_in_one_update() {
    let wanted = Wanted { memory: Target::Set(8 * GIB as i64), cpus: Target::Set(6_000_000_000) };
    // Docker reports 0 for a limit that was never set, and nothing at all for an old container's whole HostConfig.
    for old in [Some(host_config(0, 0, 0, 0)), Some(HostConfig::default()), None] {
        let update = wanted.update_from(old.as_ref()).expect("an update");
        assert_eq!((update.memory, update.memory_swap), (Some(8 * GIB as i64), Some(8 * GIB as i64)));
        assert_eq!((update.nano_cpus, update.pids_limit), (Some(6_000_000_000), Some(8192)));
    }
}

#[test]
fn a_change_sends_memory_and_swap_together_in_both_directions() {
    let two = 2 * GIB as i64;
    let eight = 8 * GIB as i64;
    for (from, to) in [(two, eight), (eight, two)] {
        let update = Wanted { memory: Target::Set(to), cpus: Target::Leave }.update_from(Some(&host_config(from, from, 0, 8192))).unwrap();
        assert_eq!((update.memory, update.memory_swap, update.nano_cpus), (Some(to), Some(to), None), "{from} to {to}");
    }
}

#[test]
fn no_limit_is_everything_the_daemon_has_and_a_limit_is_never_sent_as_zero() {
    let unlimited = ContainerLimits { memory: Limit::Unlimited, cpus: Limit::Unlimited };
    let wanted = Wanted::resolve(unlimited, ContainerLimits::default(), daemon(16, 8), None);
    assert_eq!(wanted, Wanted { memory: Target::Set(16 * GIB as i64), cpus: Target::Set(8_000_000_000) });
    let update = wanted.update_from(Some(&host_config(8 * GIB as i64, 8 * GIB as i64, 6_000_000_000, 8192))).unwrap();
    assert_eq!((update.memory, update.nano_cpus), (Some(16 * GIB as i64), Some(8_000_000_000)));
    assert!(wanted.update_from(Some(&host_config(16 * GIB as i64, 16 * GIB as i64, 8_000_000_000, 8192))).is_none());
    // Docker can't be asked: nothing is known to set, so the container is left as it is.
    let unknown = Wanted::resolve(unlimited, ContainerLimits::default(), None, None);
    assert_eq!(unknown, Wanted { memory: Target::Leave, cpus: Target::Leave });
}

#[test]
fn a_cpu_limit_above_what_docker_has_is_cut_to_it_and_memory_has_a_floor() {
    let big = ContainerLimits { memory: Limit::Fixed(1), cpus: Limit::Fixed(64_000) };
    let wanted = Wanted::resolve(big, ContainerLimits::default(), daemon(16, 8), None);
    assert_eq!(wanted, Wanted { memory: Target::Set(6 * 1024 * 1024), cpus: Target::Set(8_000_000_000) }, "6 MB is the least Docker takes");
    let unknown = Wanted::resolve(big, ContainerLimits::default(), None, None);
    assert_eq!(unknown.cpus, Target::Set(64_000_000_000), "uncut when the daemon can't be asked");
}

#[test]
fn an_unknown_target_leaves_the_container_as_it_is() {
    let wanted = Wanted { memory: Target::Leave, cpus: Target::Leave };
    assert!(wanted.update_from(Some(&host_config(3 * GIB as i64, 3 * GIB as i64, 1_000_000_000, 8192))).is_none());
}

#[test]
fn an_image_label_gives_the_recommended_minimum() {
    let labelled = |value: &str| ContainerInspectResponse {
        config: Some(bollard::models::ContainerConfig {
            labels: Some([(RECOMMENDED_MIN_MEMORY_LABEL.to_string(), value.to_string())].into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(recommended_min_mb(&labelled(" 12288 ")), Some(12288));
    assert_eq!(recommended_min_mb(&labelled("a lot")), None);
    assert_eq!(recommended_min_mb(&ContainerInspectResponse::default()), None);
}

#[test]
fn a_value_is_stored_as_text_and_a_bad_one_is_refused() {
    let n = |v: serde_json::Value| normalize(&v, "memory_mb");
    assert_eq!(n(json!("auto")).unwrap(), "");
    assert_eq!(n(json!("")).unwrap(), "");
    assert_eq!(n(json!(null)).unwrap(), "");
    assert_eq!(n(json!(0)).unwrap(), "0");
    assert_eq!(n(json!("none")).unwrap(), "0");
    assert_eq!(n(json!(4096)).unwrap(), "4096");
    assert_eq!(n(json!(" 1.5 ")).unwrap(), "1.5");
    assert!(n(json!(-1)).unwrap_err().contains("memory_mb must be a positive number"));
    assert!(n(json!("lots")).is_err());
    assert!(n(json!(true)).is_err());
}

#[test]
fn what_is_stored_reads_back_as_the_same_limits() {
    let limits = ContainerLimits::from_strings("4096", "1.5");
    assert_eq!(limits, ContainerLimits { memory: Limit::Fixed(4096), cpus: Limit::Fixed(1500) });
    assert_eq!(ContainerLimits::from_strings("", "0"), ContainerLimits { memory: Limit::Auto, cpus: Limit::Unlimited });
}
