#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for a substrate that finds its home relay through the
//! coordinator list of its registry. One substrate S has no
//! `parent_coordinator.iroh.url`. S's own registry lists nothing, so it
//! forwards `GET /coordinators` to a fake parent registry that the test
//! controls. The tests never call `home_relay().set`: the resolver owns it.

use std::{fs, time::Duration};

use anyhow::Result;
use iroh::RelayUrl;
use syneroym_core::{
    coordinator_info::CoordinatorInfo,
    curated_list::{self, CoordinatorEntry, CoordinatorTransport},
    dht_registry::RegistryClient,
};
use tokio::time;

use crate::common::{
    FakeInfo, FakeListRegistry, InfoMode, ListedRelay, NodeBuilder, SubstrateNode, alloc_ports,
    dead_relay, entry_for, list_of, listed_relay, release_held_ports, wait_for_registry_relay,
};

mod common;

/// A node with no `url` whose registry forwards list requests to `parent`.
fn resolving_builder(parent: &FakeListRegistry) -> NodeBuilder {
    let parent_url = parent.url.clone();
    SubstrateNode::builder().configure(move |c| {
        c.parent_coordinator.iroh.url = None;
        if let Some(reg) = c.roles.community_registry.as_mut() {
            reg.parent_registry_url = Some(parent_url.clone());
        }
    })
}

fn registry_client(node: &SubstrateNode) -> RegistryClient {
    RegistryClient::new(false, Some(node.registry_url().to_string()))
}

/// The relay the rank gives `did`, as a normalised URL.
fn first_ranked(did: &str, entries: &[CoordinatorEntry]) -> String {
    curated_list::rank(did, entries.iter().cloned()).remove(0).url
}

/// The normalised relay URL of a listed relay.
fn normal_url(relay: &ListedRelay) -> String {
    curated_list::normalize_url(relay.relay_url.as_str()).unwrap()
}

fn entries_of(relays: &[&ListedRelay]) -> Vec<CoordinatorEntry> {
    relays.iter().map(|r| r.entry.clone()).collect()
}

/// The saved list and choice of `node`.
fn read_cache(node: &SubstrateNode) -> serde_json::Value {
    let path = node.base_path().join("data/db/coordinators.json");
    serde_json::from_slice(&fs::read(path).expect("coordinators.json exists")).unwrap()
}

#[tokio::test]
async fn picks_a_listed_relay_and_publishes_it() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let (a, b) = (listed_relay(InfoMode::Healthy).await?, listed_relay(InfoMode::Healthy).await?);
    let entries = entries_of(&[&a, &b]);
    let parent = FakeListRegistry::spawn(Some(list_of(&entries, 3600))).await?;

    let node = resolving_builder(&parent).boot().await;

    let expected = first_ranked(node.did(), &entries);
    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        &expected,
        Duration::from_secs(30),
    )
    .await?;
    assert_eq!(read_cache(&node)["chosen"]["url"], expected.as_str());
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn skips_coordinators_that_are_full_or_down() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let full = listed_relay(InfoMode::AtCapacity).await?;
    let down = listed_relay(InfoMode::Unavailable).await?;
    let good = listed_relay(InfoMode::Healthy).await?;
    let entries = entries_of(&[&full, &down, &good]);
    let parent = FakeListRegistry::spawn(Some(list_of(&entries, 3600))).await?;

    let node = resolving_builder(&parent).boot().await;

    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        good.relay_url.as_str(),
        Duration::from_secs(30),
    )
    .await?;
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn restart_uses_the_saved_choice() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let (a, b) = (listed_relay(InfoMode::Healthy).await?, listed_relay(InfoMode::Healthy).await?);
    let entries = entries_of(&[&a, &b]);
    let parent = FakeListRegistry::spawn(Some(list_of(&entries, 3600))).await?;

    let node = resolving_builder(&parent).boot().await;
    let expected = first_ranked(node.did(), &entries);
    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        &expected,
        Duration::from_secs(30),
    )
    .await?;
    let requests = parent.requests();

    let node = node.restart().await;

    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        &expected,
        Duration::from_secs(30),
    )
    .await?;
    assert_eq!(parent.requests(), requests, "the saved choice needs no list request");
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn expired_list_is_used_when_the_registry_is_down() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let (a, b) = (listed_relay(InfoMode::Healthy).await?, listed_relay(InfoMode::Healthy).await?);
    let entries = entries_of(&[&a, &b]);
    let parent = FakeListRegistry::spawn(Some(list_of(&entries, 1))).await?;

    let node = resolving_builder(&parent).boot().await;
    let first = first_ranked(node.did(), &entries);
    wait_for_registry_relay(&registry_client(&node), node.did(), &first, Duration::from_secs(30))
        .await?;
    let (chosen, other) = if normal_url(&a) == first { (&a, &b) } else { (&b, &a) };

    // The chosen relay stops answering and the registry is down.
    chosen.info.set(InfoMode::Unavailable);
    parent.set(None);
    time::sleep(Duration::from_secs(2)).await;
    let node = node.restart().await;

    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        other.relay_url.as_str(),
        Duration::from_secs(60),
    )
    .await?;
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn relay_that_leaves_the_list_is_replaced() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let (a, b) = (listed_relay(InfoMode::Healthy).await?, listed_relay(InfoMode::Healthy).await?);
    let parent = FakeListRegistry::spawn(Some(list_of(&entries_of(&[&a]), 1))).await?;

    let node = resolving_builder(&parent).boot().await;
    let client = registry_client(&node);
    wait_for_registry_relay(&client, node.did(), a.relay_url.as_str(), Duration::from_secs(30))
        .await?;

    parent.set(Some(list_of(&entries_of(&[&b]), 1)));

    // The next refresh is at least 10 s away, and a switch can take 15 s.
    wait_for_registry_relay(&client, node.did(), b.relay_url.as_str(), Duration::from_secs(40))
        .await?;
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn degraded_until_the_registry_answers() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let a = listed_relay(InfoMode::Healthy).await?;
    let parent = FakeListRegistry::spawn(None).await?;

    let mut node = resolving_builder(&parent).skip_ready_wait().boot().await;

    time::sleep(Duration::from_secs(3)).await;
    let client = registry_client(&node);
    assert!(client.lookup(node.did(), false).await.is_err(), "no record before a home relay");
    assert_eq!(node.home_relay().current_url(), None);

    parent.set(Some(list_of(&entries_of(&[&a]), 3600)));

    wait_for_registry_relay(&client, node.did(), a.relay_url.as_str(), Duration::from_secs(40))
        .await?;
    node.substrate_client.wait_for_ready(Duration::from_secs(30)).await?;
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn fixed_url_never_asks_the_registry() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let a = listed_relay(InfoMode::Healthy).await?;
    let parent = FakeListRegistry::spawn(Some(list_of(&entries_of(&[&a]), 3600))).await?;
    let parent_url = parent.url.clone();

    // The harness leaves `url` set to the node's own relay.
    let node = SubstrateNode::builder()
        .configure(move |c| {
            if let Some(reg) = c.roles.community_registry.as_mut() {
                reg.parent_registry_url = Some(parent_url.clone());
            }
        })
        .boot()
        .await;

    // The node's own registry may need a few seconds to bind. Wait longer
    // than its retries, so a resolver that started by mistake would have
    // asked by now.
    time::sleep(Duration::from_secs(6)).await;
    assert_eq!(parent.requests(), 0);
    assert!(
        !node.base_path().join("data/db/coordinators.json").exists(),
        "no resolver ran, so no cache file exists"
    );
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn relay_that_does_not_confirm_is_skipped() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let live = listed_relay(InfoMode::Healthy).await?;
    // Its info answers, but its relay accepts TCP and never speaks.
    let (_dead_listener, dead_url) = dead_relay().await?;
    let dead_info = FakeInfo::spawn(InfoMode::Healthy).await?;
    let dead_entry = entry_for(dead_url.as_str(), &dead_info);
    let entries = vec![live.entry.clone(), dead_entry];
    let parent = FakeListRegistry::spawn(Some(list_of(&entries, 3600))).await?;

    let node = resolving_builder(&parent).boot().await;

    // At worst the dead relay is tried first and the confirm waits 15 s.
    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        live.relay_url.as_str(),
        Duration::from_secs(45),
    )
    .await?;
    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn uses_a_real_coordinator_from_the_list() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    // A coordinator with a known info port: the real `/v1/info` is probed.
    let [info_port] = alloc_ports::<1>();
    release_held_ports(&[info_port]);
    let coordinator = SubstrateNode::builder()
        .configure(move |c| {
            if let Some(iroh) = c.roles.coordinator.as_mut().and_then(|r| r.iroh.as_mut()) {
                iroh.info_http_bind_address = Some(format!("127.0.0.1:{info_port}"));
            }
        })
        .boot()
        .await;
    let entry = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: coordinator.relay_url().to_owned(),
        info_url: format!("http://127.0.0.1:{info_port}"),
        operator: "real".into(),
    };
    let parent = FakeListRegistry::spawn(Some(list_of(&[entry], 3600))).await?;

    let node = resolving_builder(&parent).boot().await;

    wait_for_registry_relay(
        &registry_client(&node),
        node.did(),
        coordinator.relay_url(),
        Duration::from_secs(30),
    )
    .await?;
    node.teardown().await;
    coordinator.teardown().await;
    Ok(())
}

#[tokio::test]
async fn coordinator_role_never_asks_for_the_list() -> Result<()> {
    let _serial_guard = common::serial_guard().await;
    let a = listed_relay(InfoMode::Healthy).await?;
    let parent = FakeListRegistry::spawn(Some(list_of(&entries_of(&[&a]), 3600))).await?;
    let [info_port] = alloc_ports::<1>();
    release_held_ports(&[info_port]);
    let parent_url = parent.url.clone();

    // No Iroh interface: the router builds no endpoint, so no resolver runs.
    // Only the coordinator role is left to ask for a list, and it must not.
    let node = SubstrateNode::builder()
        .configure(move |c| {
            c.parent_coordinator.iroh.url = None;
            c.substrate.communication_interfaces = Vec::new();
            if let Some(reg) = c.roles.community_registry.as_mut() {
                reg.parent_registry_url = Some(parent_url.clone());
            }
            if let Some(iroh) = c.roles.coordinator.as_mut().and_then(|r| r.iroh.as_mut()) {
                iroh.info_http_bind_address = Some(format!("127.0.0.1:{info_port}"));
            }
        })
        .skip_ready_wait()
        .no_owner_client()
        .boot()
        .await;

    time::sleep(Duration::from_secs(5)).await;

    assert_eq!(parent.requests(), 0);
    let info: CoordinatorInfo = reqwest::get(format!("http://127.0.0.1:{info_port}/v1/info"))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let own_relay: RelayUrl = node.relay_url().parse()?;
    assert_eq!(info.relay_url.map(|u| u.parse::<RelayUrl>().unwrap()), Some(own_relay));
    node.teardown().await;
    Ok(())
}
