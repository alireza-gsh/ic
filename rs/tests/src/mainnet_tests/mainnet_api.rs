use std::path::Path;

use crate::driver::test_env::TestEnvAttribute;
use crate::driver::test_env_api::{HasPublicApiUrl, HasRegistryLocalStore};
use crate::driver::test_setup::GroupSetup;
use crate::driver::{
    test_env::{HasIcPrepDir, TestEnv},
    test_env_api::{HasTopologySnapshot, IcNodeContainer},
};
use ic_prep_lib::prep_state_directory::IcPrepStateDir;
use slog::info;

use ic_registry_local_store::{compact_delta_to_changelog, Changelog, LocalStoreImpl};
use ic_registry_local_store_artifacts;

const PRODUCTION_IC_NAME: &str = "mercury";

const MIN_EXPECTED_NUM_SUBNETS: usize = 30;

pub fn get_mainnet_delta_6d_c1() -> Changelog {
    compact_delta_to_changelog(ic_registry_local_store_artifacts::MAINNET_DELTA_00_6D_C1)
        .expect("Could not read mainnet delta 00-6d-c1")
        .1
}

/// this method uses unsafe writes. as a result, after a power outage, it is
/// possible that the latest version is lower than the highest version in
/// `changelog`, even if the call returned successfully. we deem this acceptable
/// in this particular case, as this method is only used on startup and the
/// missing versions will be fetched through subsequent updates of the local
/// store.
pub fn create_local_store_from_changelog<P: AsRef<Path>>(
    path: P,
    changelog: Changelog,
) -> LocalStoreImpl {
    let store = LocalStoreImpl::new(path.as_ref());
    for (v, changelog_entry) in changelog.into_iter().enumerate() {
        store
            .write_changelog_entry_unsafe((v + 1) as u64, changelog_entry)
            .unwrap();
    }
    store
}

pub fn mainnet_config(env: TestEnv) {
    let log = env.logger();

    let group_setup = GroupSetup::from_bazel_env();
    group_setup.write_attribute(&env);
    info!(&log, "Created group_setup directory");

    let target_dir = env.create_prep_dir(PRODUCTION_IC_NAME).unwrap();

    let abs_target_dir = std::env::current_exe().expect("could not acquire executable directory");
    info!(
        &log,
        "Created ic_prep directory: {:?}",
        &abs_target_dir.join(target_dir.path())
    );

    let local_store_path = env
        .registry_local_store_path(PRODUCTION_IC_NAME)
        .expect("corrupted ic-prep directory structure");
    let _store =
        create_local_store_from_changelog(local_store_path.clone(), get_mainnet_delta_6d_c1());
    info!(&log, "Obtained old registry snapshot");

    let topology_snapshot = env.topology_snapshot_by_name(PRODUCTION_IC_NAME);
    let rt = tokio::runtime::Runtime::new().expect("Could not create runtime");
    let _topology_snapshot = rt
        .block_on(topology_snapshot.block_for_newest_mainnet_registry_version())
        .unwrap();
    info!(&log, "Fast-forward to the latest registry snapshot");

    let reg_snapshot = ic_regedit::load_registry_local_store(local_store_path).unwrap();
    let reg_snapshot_serialized =
        serde_json::to_string_pretty(&reg_snapshot).expect("Could not pretty print value.");
    IcPrepStateDir::new(target_dir.path());
    std::fs::write(
        target_dir.path().join("initial_registry_snapshot.json"),
        reg_snapshot_serialized,
    )
    .unwrap();
    info!(
        &log,
        "Saved production IC registry snapshot as initial_registry_snapshot.json"
    );
}

pub fn mainnet_basic_test(env: TestEnv) {
    let log = env.logger();
    let nodes: Vec<_> = env
        .topology_snapshot_by_name(PRODUCTION_IC_NAME)
        .subnets()
        .map(|s| s.nodes().next().unwrap())
        .collect();

    assert!(nodes.len() >= MIN_EXPECTED_NUM_SUBNETS);

    info!(
        log,
        "Waiting for {} nodes to become healthy ...",
        nodes.len()
    );
    nodes
        .iter()
        .try_for_each(|n| n.await_status_is_healthy())
        .unwrap();

    info!(log, "Each subnet contains at least one healthy node!");
}
