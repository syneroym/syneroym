use clap::{Parser, error::ErrorKind};
use syneroym_sdk::SyneroymClient;

use super::*;

#[derive(Debug, Parser)]
struct Wrapper {
    #[command(subcommand)]
    cmd: SvcCommands,
}

/// `--instance-certificate` installs an already-minted certificate
/// as-is; `--master` mints and installs a fresh one itself.
/// Together they're ambiguous about which certificate actually gets
/// installed, so clap must reject the combination before either flag's
/// handler ever runs.
#[test]
fn deploy_rejects_master_and_instance_certificate_together() {
    let err = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--tcp",
        "localhost:1",
        "--master",
        "m",
        "--instance-certificate",
        "/tmp/cert.json",
    ])
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
}

/// `--wasm` and `--image` pick two different deploy kinds for the same
/// service -- clap must reject the combination, not silently prefer one
/// via if/else precedence in the handler.
#[test]
fn deploy_rejects_wasm_and_image_together() {
    let err = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--wasm",
        "/tmp/app.wasm",
        "--image",
        "docker.io/library/nginx:alpine",
    ])
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
}

/// Same as `deploy_rejects_wasm_and_image_together`, for the other pair
/// of deploy-kind flags.
#[test]
fn deploy_rejects_tcp_and_image_together() {
    let err = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--tcp",
        "localhost:1",
        "--image",
        "docker.io/library/nginx:alpine",
    ])
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
}

/// `--port`/`--volume` only mean anything alongside `--image`. clap
/// cannot reject this combination itself (see `validate_container_flags`
/// for why), so `--tcp ... --port ...` parses fine at the clap layer --
/// it is `validate_container_flags`, called from the handler before
/// either arm runs, that must catch it instead.
#[test]
fn deploy_with_tcp_and_port_parses_but_is_not_a_valid_combination() {
    let cmd = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--tcp",
        "localhost:1",
        "--port",
        "default:80:8080",
    ])
    .expect("clap itself does not reject --tcp with --port");
    let SvcCommands::Deploy { image, ports, .. } = cmd.cmd else {
        panic!("expected SvcCommands::Deploy");
    };
    assert!(image.is_none());
    assert_eq!(
        validate_container_flags(&image, &ports, &[]).unwrap_err().to_string(),
        "--port/--volume require --image"
    );
}

#[test]
fn validate_container_flags_rejects_ports_without_image() {
    let ports = vec!["default:80:8080".to_string()];
    let err = validate_container_flags(&None, &ports, &[]).unwrap_err();
    assert!(err.to_string().contains("--port/--volume require --image"));
}

#[test]
fn validate_container_flags_rejects_volumes_without_image() {
    let volumes = vec!["html:/usr/share/nginx/html".to_string()];
    let err = validate_container_flags(&None, &[], &volumes).unwrap_err();
    assert!(err.to_string().contains("--port/--volume require --image"));
}

#[test]
fn validate_container_flags_accepts_ports_and_volumes_with_image() {
    let image = Some("docker.io/library/nginx:alpine".to_string());
    let ports = vec!["default:80:8080".to_string()];
    let volumes = vec!["html:/usr/share/nginx/html".to_string()];
    validate_container_flags(&image, &ports, &volumes).unwrap();
}

#[test]
fn validate_container_flags_accepts_image_with_no_ports_or_volumes() {
    let image = Some("docker.io/library/nginx:alpine".to_string());
    validate_container_flags(&image, &[], &[]).unwrap();
}

/// `--master` plus `--nickname` needs no `--identity` -- the master
/// identity already loaded on this arm carries the envelope.
#[test]
fn deploy_with_a_master_and_a_nickname_needs_no_identity_flag() {
    let cmd = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--tcp",
        "localhost:1",
        "--master",
        "m",
        "--nickname",
        "alice",
    ])
    .expect("--master with --nickname and no --identity must parse");
    let SvcCommands::Deploy { master, nickname, identity, .. } = cmd.cmd else {
        panic!("expected SvcCommands::Deploy");
    };
    assert_eq!(master.as_deref(), Some("m"));
    assert_eq!(nickname.as_deref(), Some("alice"));
    assert!(identity.is_none());
}

/// `--instance-certificate` alone (no `--identity`/`--master`) parses
/// fine with a `--nickname`, even though there is no local key that
/// could sign a record verifying under `svc_id` -- the nickname is
/// silently dropped with a warning at runtime, not rejected at parse
/// time.
#[test]
fn deploy_with_an_instance_certificate_and_a_nickname_needs_no_identity_flag() {
    let cmd = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--tcp",
        "localhost:1",
        "--instance-certificate",
        "/tmp/cert.json",
        "--nickname",
        "alice",
    ])
    .expect("--instance-certificate with --nickname and no --identity must parse");
    let SvcCommands::Deploy { instance_certificate, nickname, identity, .. } = cmd.cmd else {
        panic!("expected SvcCommands::Deploy");
    };
    assert_eq!(instance_certificate, Some(PathBuf::from("/tmp/cert.json")));
    assert_eq!(nickname.as_deref(), Some("alice"));
    assert!(identity.is_none());
}

/// `--image` (with repeatable `--port`/`--volume`) parses and reaches
/// the container arm, mirroring `--wasm`/`--tcp`'s own presence-based
/// dispatch -- `wasm`/`tcp` stay `None` since only `--image` was given.
#[test]
fn deploy_with_image_port_and_volume_reaches_the_container_arm() {
    let cmd = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--image",
        "docker.io/library/nginx:alpine",
        "--port",
        "default:80:8080:tcp",
        "--volume",
        "html:/usr/share/nginx/html",
    ])
    .expect("--image with --port and --volume must parse");
    let SvcCommands::Deploy { image, ports, volumes, wasm, tcp, .. } = cmd.cmd else {
        panic!("expected SvcCommands::Deploy");
    };
    assert_eq!(image.as_deref(), Some("docker.io/library/nginx:alpine"));
    assert_eq!(ports, vec!["default:80:8080:tcp".to_string()]);
    assert_eq!(volumes, vec!["html:/usr/share/nginx/html".to_string()]);
    assert!(wasm.is_none());
    assert!(tcp.is_none());
}

/// `--master` with `--image` parses, mirroring
/// `deploy_with_a_master_and_a_nickname_needs_no_identity_flag` for the
/// TCP arm -- a member master applies identically regardless of which
/// service type is being deployed.
#[test]
fn deploy_with_a_master_and_an_image_parses() {
    let cmd = Wrapper::try_parse_from([
        "svc",
        "deploy",
        "--svc-id",
        "did:key:zTest",
        "--interfaces",
        "default",
        "--image",
        "docker.io/library/nginx:alpine",
        "--master",
        "m",
    ])
    .expect("--master with --image must parse");
    let SvcCommands::Deploy { master, image, .. } = cmd.cmd else {
        panic!("expected SvcCommands::Deploy");
    };
    assert_eq!(master.as_deref(), Some("m"));
    assert_eq!(image.as_deref(), Some("docker.io/library/nginx:alpine"));
}

#[test]
fn parse_container_port_mapping_rejects_malformed_input() {
    let err = parse_container_port_mapping("default").unwrap_err();
    assert!(err.to_string().contains("Invalid --port"));
}

#[test]
fn parse_container_port_mapping_defaults_host_port_and_protocol() {
    let mapping = parse_container_port_mapping("default:80").unwrap();
    assert_eq!(mapping.interface_name, "default");
    assert_eq!(mapping.container_port, 80);
    assert_eq!(mapping.host_port, None);
    assert_eq!(mapping.protocol, "tcp");
}

#[test]
fn parse_container_port_mapping_parses_host_port_and_protocol() {
    let mapping = parse_container_port_mapping("default:80:8080:udp").unwrap();
    assert_eq!(mapping.host_port, Some(8080));
    assert_eq!(mapping.protocol, "udp");
}

#[test]
fn parse_container_port_mapping_allows_an_empty_host_port_with_a_protocol() {
    let mapping = parse_container_port_mapping("default:80::udp").unwrap();
    assert_eq!(mapping.host_port, None);
    assert_eq!(mapping.protocol, "udp");
}

#[test]
fn parse_container_port_mapping_rejects_an_empty_interface_name() {
    let err = parse_container_port_mapping(":80:8080").unwrap_err();
    assert!(err.to_string().contains("interface name must not be empty"));
}

#[test]
fn parse_container_port_mapping_rejects_an_unsupported_protocol() {
    let err = parse_container_port_mapping("default:80:8080:tpc").unwrap_err();
    assert!(err.to_string().contains("protocol 'tpc' is not one of"));
}

#[test]
fn parse_container_port_mapping_rejects_too_many_segments() {
    let err = parse_container_port_mapping("default:80:8080:tcp:extra").unwrap_err();
    assert!(err.to_string().contains("Invalid --port"));
}

#[test]
fn parse_container_port_mapping_rejects_a_non_numeric_container_port() {
    let err = parse_container_port_mapping("default:not-a-port").unwrap_err();
    assert!(err.to_string().contains("Invalid container_port"));
}

#[test]
fn parse_container_volume_mapping_rejects_malformed_input() {
    let err = parse_container_volume_mapping("no-colon-here").unwrap_err();
    assert!(err.to_string().contains("Invalid --volume"));
}

#[test]
fn parse_container_volume_mapping_parses_host_and_container_path() {
    let mapping = parse_container_volume_mapping("html:/usr/share/nginx/html").unwrap();
    assert_eq!(mapping.host_path, "html");
    assert_eq!(mapping.container_path, "/usr/share/nginx/html");
    assert!(mapping.files.is_empty());
}

/// Docker/Podman users commonly type a trailing mount option like
/// `:ro` -- rejecting it with a clear message beats silently folding it
/// into `container_path` (`/var/lib/data:ro`, which podman would then
/// fail on with a much more confusing error).
#[test]
fn parse_container_volume_mapping_rejects_a_docker_style_mount_option() {
    let err = parse_container_volume_mapping("/data:/var/lib/data:ro").unwrap_err();
    assert!(err.to_string().contains("mount options"));
}

#[test]
fn validate_container_ports_rejects_no_ports() {
    let ifaces = vec!["default".to_string()];
    let err = validate_container_ports(&ifaces, &[]).unwrap_err();
    assert!(err.to_string().contains("at least one --port"));
}

#[test]
fn validate_container_ports_rejects_a_port_interface_absent_from_interfaces() {
    let ifaces = vec!["default".to_string()];
    let mapping = parse_container_port_mapping("other:80").unwrap();
    let err = validate_container_ports(&ifaces, &[mapping]).unwrap_err();
    assert!(err.to_string().contains("not in --interfaces"));
}

#[test]
fn validate_container_ports_accepts_a_port_naming_a_declared_interface() {
    let ifaces = vec!["default".to_string()];
    let mapping = parse_container_port_mapping("default:80").unwrap();
    validate_container_ports(&ifaces, &[mapping]).unwrap();
}

/// A blank `--interfaces` composes correctly with `--port` for a
/// container deploy too, not only `--wasm`/`--tcp`: `parse_interfaces`
/// falls back to `["default"]`, and a `--port default:80` mapping
/// names exactly that.
#[test]
fn a_blank_interfaces_value_composes_with_a_default_named_container_port() {
    let ifaces = parse_interfaces("").unwrap();
    let mapping = parse_container_port_mapping("default:80").unwrap();
    validate_container_ports(&ifaces, &[mapping]).unwrap();
}

/// The bug this replaces: `"".split(',')` yields one empty-string
/// element, never zero, so a length check could never see this case
/// as "empty" -- `--interfaces ""` used to silently register a
/// service under the literal interface name `""`.
#[test]
fn parse_interfaces_falls_back_to_the_shared_default_name_when_blank() {
    assert_eq!(parse_interfaces("").unwrap(), vec![DEFAULT_INTERFACE_NAME.to_string()]);
    assert_eq!(parse_interfaces("   ").unwrap(), vec![DEFAULT_INTERFACE_NAME.to_string()]);
}

#[test]
fn parse_interfaces_splits_and_trims_a_real_list() {
    assert_eq!(
        parse_interfaces("http, admin").unwrap(),
        vec!["http".to_string(), "admin".to_string()]
    );
}

/// A blank *segment* amid otherwise real names (a stray comma) is a
/// different mistake than an omitted value entirely, and is refused
/// rather than silently coerced to the default.
#[test]
fn parse_interfaces_rejects_a_blank_segment_in_an_otherwise_real_list() {
    let err = parse_interfaces("http,,admin").unwrap_err();
    assert!(err.to_string().contains("position 2"), "{err}");

    let err = parse_interfaces(",admin").unwrap_err();
    assert!(err.to_string().contains("position 1"), "{err}");
}

#[test]
fn format_expiry_shows_a_dash_for_no_certificate() {
    assert_eq!(format_expiry(None), "-");
}

#[test]
fn format_expiry_shows_an_rfc3339_timestamp() {
    // 2024-01-01T00:00:00Z
    assert_eq!(format_expiry(Some(1_704_067_200)), "2024-01-01T00:00:00+00:00");
}

#[test]
fn parse_visibility_accepts_the_three_declared_values_case_insensitively() {
    assert_eq!(parse_visibility("public", "--visibility").unwrap(), Visibility::Public);
    assert_eq!(parse_visibility("Internal", "--visibility").unwrap(), Visibility::Internal);
    assert_eq!(parse_visibility("PRIVATE", "--visibility").unwrap(), Visibility::Private);
}

#[test]
fn parse_visibility_rejects_an_unknown_value_naming_the_flag() {
    let err = parse_visibility("hidden", "--visibility").unwrap_err();
    assert!(err.to_string().contains("--visibility"), "{err}");
    assert!(err.to_string().contains("hidden"), "{err}");

    let err = parse_visibility("hidden", "--asset-visibility").unwrap_err();
    assert!(err.to_string().contains("--asset-visibility"), "{err}");
}

/// `signed_export_record` is the exact function both the
/// private-with-`--record-out` deploy arm and the public/internal arm
/// call to build the record they sign -- a regression here is a
/// regression in `svc deploy` itself, not just in this test's own copy
/// of the shape.
#[test]
fn signed_export_record_sets_is_private_from_visibility() {
    let identity = Identity::generate().unwrap();
    let svc_id = substrate::derive_did_key(&identity.public_key());
    for (v, expect_private) in
        [(Visibility::Private, true), (Visibility::Internal, true), (Visibility::Public, false)]
    {
        let record =
            signed_export_record(v, &svc_id, "did:key:zSubstrate", None, 9_999_999_999, &identity)
                .unwrap();
        assert_eq!(record.info.is_private, expect_private, "{v:?}");
    }
}

/// The `--record-out` -> file -> `new_with_record` round trip
/// (ADR-0018 §2), over the record `signed_export_record` actually
/// builds -- `new_with_record_verifies_signature_and_sets_fields`
/// (`crates/sdk/src/lib.rs`) already covers the signature-verification
/// half against an in-memory record; what is missing is the file itself
/// (`svc deploy` cannot be driven from a test -- `roymctl` is a binary,
/// not linkable).
#[test]
fn a_record_out_file_round_trips_through_new_with_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("svc.record.json");

    let identity = Identity::generate().unwrap();
    let svc_id = substrate::derive_did_key(&identity.public_key());
    let record = signed_export_record(
        Visibility::Private,
        &svc_id,
        "did:key:zSubstrate",
        Some("my-private-svc".to_string()),
        9_999_999_999,
        &identity,
    )
    .unwrap();

    fs::write(&path, serde_json::to_string_pretty(&record).unwrap()).unwrap();

    let read_back: SignedEndpointInfo =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let client =
        SyneroymClient::new_with_record(read_back, "http://127.0.0.1:9999".to_string()).unwrap();
    assert_eq!(client.service_id(), svc_id);
}
