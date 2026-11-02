use std::time::Duration;

use syneroym_core::config::{AppSandboxRole, SubstrateConfig};

use crate::host_capabilities::{CallAll, CallAllLimits};

#[test]
fn defaults_match_specification() {
    let config = SubstrateConfig::default();
    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits, CallAllLimits::default());
    assert_eq!(call_all.limits.max_calls, 16);
    assert_eq!(call_all.limits.max_concurrency, 8);
    assert_eq!(call_all.limits.default_call_timeout, Duration::from_millis(5000));
    assert_eq!(call_all.limits.max_call_timeout, Duration::from_millis(10000));
    assert_eq!(call_all.limits.total_timeout, Duration::from_millis(15000));
}

#[test]
fn zero_values_clamped_to_one() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole::default());
    let sandbox = config.roles.app_sandbox.as_mut().unwrap();
    sandbox.call_all_max_calls = 0;
    sandbox.call_all_max_concurrency = 0;
    sandbox.call_all_default_call_timeout_ms = 0;
    sandbox.call_all_max_call_timeout_ms = 0;
    sandbox.call_all_total_timeout_ms = 0;
    sandbox.call_all_max_waiting_guests = 0;

    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits.max_calls, 1);
    assert_eq!(call_all.limits.max_concurrency, 1);
    assert_eq!(call_all.limits.default_call_timeout, Duration::from_millis(1));
    assert_eq!(call_all.limits.max_call_timeout, Duration::from_millis(1));
    assert_eq!(call_all.limits.total_timeout, Duration::from_millis(1000));
}

#[test]
fn total_timeout_below_min_batch_allowance_clamped() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole::default());
    let sandbox = config.roles.app_sandbox.as_mut().unwrap();
    sandbox.call_all_total_timeout_ms = 800;

    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits.total_timeout, Duration::from_millis(1000));
}

#[test]
fn default_call_timeout_clamped_to_max_call_timeout() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole::default());
    let sandbox = config.roles.app_sandbox.as_mut().unwrap();
    sandbox.call_all_default_call_timeout_ms = 12000;
    sandbox.call_all_max_call_timeout_ms = 8000;
    sandbox.call_all_total_timeout_ms = 15000;

    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits.default_call_timeout, Duration::from_millis(8000));
    assert_eq!(call_all.limits.max_call_timeout, Duration::from_millis(8000));
}

#[test]
fn max_call_timeout_clamped_to_total_timeout() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole::default());
    let sandbox = config.roles.app_sandbox.as_mut().unwrap();
    sandbox.call_all_default_call_timeout_ms = 5000;
    sandbox.call_all_max_call_timeout_ms = 20000;
    sandbox.call_all_total_timeout_ms = 12000;

    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits.max_call_timeout, Duration::from_millis(12000));
    assert_eq!(call_all.limits.total_timeout, Duration::from_millis(12000));
}

#[test]
fn max_waiting_guests_clamped_to_instances_minus_one() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole::default());
    let sandbox = config.roles.app_sandbox.as_mut().unwrap();
    sandbox.max_concurrent_instances = 4;
    sandbox.call_all_max_waiting_guests = 10;

    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.waiters.available_permits(), 3);
}

#[test]
fn config_file_without_call_all_fields_loads_successfully() {
    let json = r#"
    {
        "roles": {
            "app_sandbox": {
                "max_concurrent_instances": 10
            }
        }
    }
    "#;
    let config = serde_json::from_str::<SubstrateConfig>(json).unwrap();
    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits, CallAllLimits::default());
    assert_eq!(call_all.waiters.available_permits(), 4);
}

#[test]
fn config_without_app_sandbox_role_uses_defaults() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = None;
    let call_all = CallAll::from_config(&config);
    assert_eq!(call_all.limits, CallAllLimits::default());
    assert_eq!(call_all.waiters.available_permits(), 4);
}
