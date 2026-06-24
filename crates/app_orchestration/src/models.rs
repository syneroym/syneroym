use std::{collections::BTreeMap, fmt, str::FromStr};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

macro_rules! define_string_wrapper {
    ($name:ident, $doc:expr) => {
        define_string_wrapper!($name, $doc, |s: &str| {
            if s.is_empty() {
                Err(anyhow!("{} cannot be empty", stringify!($name)))
            } else {
                Ok(())
            }
        });
    };
    ($name:ident, $doc:expr, $validate:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn try_new<S: Into<String>>(s: S) -> Result<Self> {
                let s = s.into();
                let validator: fn(&str) -> Result<()> = $validate;
                validator(&s)?;
                Ok($name(s))
            }

            pub fn new<S: Into<String>>(s: S) -> Self {
                Self::try_new(s)
                    .unwrap_or_else(|e| panic!("Invalid value for {}: {}", stringify!($name), e))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = anyhow::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::try_new(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = anyhow::Error;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::try_new(s)
            }
        }

        impl From<$name> for String {
            fn from(wrapper: $name) -> Self {
                wrapper.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = String;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

define_string_wrapper!(
    AppBlueprintId,
    "Unique identifier for an application blueprint/definition."
);
define_string_wrapper!(AppInstanceId, "Unique identifier for a running application instance.");

define_string_wrapper!(
    LogicalServiceName,
    "Logical name of a service within an application.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("LogicalServiceName cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("LogicalServiceName cannot contain '/'"));
        }
        Ok(())
    }
);

define_string_wrapper!(
    ServiceId,
    "Physical identifier of a service (usually a did:key DID).",
    |s: &str| {
        if !s.starts_with("did:key:") {
            return Err(anyhow!("ServiceId must start with 'did:key:'"));
        }
        Ok(())
    }
);

define_string_wrapper!(InterfaceName, "Name of the interface a service implements.");
define_string_wrapper!(DependencyName, "Name of a dependency within an application.");

/// Logical reference to a service, fully identifying it within a specific
/// application instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LogicalServiceRef {
    pub app_instance_id: AppInstanceId,
    pub service_name: LogicalServiceName,
}

impl fmt::Display for LogicalServiceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.app_instance_id, self.service_name)
    }
}

impl FromStr for LogicalServiceRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(anyhow!("LogicalServiceRef must format as 'app_instance_id/service_name'"));
        }
        let app_instance_id = AppInstanceId::try_new(parts[0])?;
        let service_name = LogicalServiceName::try_new(parts[1])?;
        Ok(LogicalServiceRef { app_instance_id, service_name })
    }
}

impl TryFrom<String> for LogicalServiceRef {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl From<LogicalServiceRef> for String {
    fn from(r: LogicalServiceRef) -> Self {
        r.to_string()
    }
}

/// Supported service execution types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    Wasm,
    Container,
    Tcp,
    #[serde(rename = "nativehost")]
    NativeHost,
}

/// Service topology deployment modes.
#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum TopologyMode {
    #[default]
    Singleton,
    Redundant,
    Sharded,
}

/// Shared execution configuration for a service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceConfig {
    pub service_type: ServiceType,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<InterfaceName>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_config: Option<String>,
}

/// Represents the spec of a service in the application manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceSpec {
    #[serde(flatten)]
    pub config: ServiceConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<LogicalServiceName>,
}

/// Defines a dependency on another application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum AppDependencySpec {
    Spawn { blueprint: AppBlueprintId, manifest_path: Option<String> },
    Bind { instance: AppInstanceId },
}

/// Declarative manifest specifying the structure and dependencies of a SynApp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SynAppManifest {
    pub id: AppBlueprintId,
    pub version: semver::Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub services: BTreeMap<LogicalServiceName, ServiceSpec>,
    #[serde(default)]
    pub dependencies: BTreeMap<DependencyName, AppDependencySpec>,
}

impl SynAppManifest {
    pub fn from_toml(s: &str) -> Result<Self> {
        let manifest: Self =
            toml::from_str(s).map_err(|e| anyhow!("Failed to parse TOML manifest: {}", e))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(s).map_err(|e| anyhow!("Failed to parse JSON manifest: {}", e))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| anyhow!("Failed to serialize to TOML manifest: {}", e))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize to JSON manifest: {}", e))
    }

    pub fn validate(&self) -> Result<()> {
        // 1. Verify that depends_on references actual services within the manifest.
        for (name, spec) in &self.services {
            for dep in &spec.depends_on {
                if !self.services.contains_key(dep) {
                    return Err(anyhow!(
                        "Service '{}' depends on undefined service '{}'",
                        name,
                        dep
                    ));
                }
            }
        }

        // 2. Perform cycle detection.
        let mut visited = BTreeMap::new();
        let mut stack = BTreeMap::new();
        for name in self.services.keys() {
            visited.insert(name, false);
            stack.insert(name, false);
        }

        fn has_cycle<'a>(
            node: &'a LogicalServiceName,
            services: &'a BTreeMap<LogicalServiceName, ServiceSpec>,
            visited: &mut BTreeMap<&'a LogicalServiceName, bool>,
            stack: &mut BTreeMap<&'a LogicalServiceName, bool>,
        ) -> bool {
            if *stack.get(node).unwrap_or(&false) {
                return true;
            }
            if *visited.get(node).unwrap_or(&false) {
                return false;
            }

            visited.insert(node, true);
            stack.insert(node, true);

            if let Some(spec) = services.get(node) {
                for dep in &spec.depends_on {
                    if has_cycle(dep, services, visited, stack) {
                        return true;
                    }
                }
            }

            stack.insert(node, false);
            false
        }

        for name in self.services.keys() {
            if has_cycle(name, &self.services, &mut visited, &mut stack) {
                return Err(anyhow!("Circular dependency detected in services"));
            }
        }

        Ok(())
    }
}

/// A planned, compiled service instance within a deployment plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedService {
    pub service_id: ServiceId,
    pub logical_ref: LogicalServiceRef,
    #[serde(flatten)]
    pub config: ServiceConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_dependencies: Vec<ServiceId>,
    #[serde(default)]
    pub topology_mode: TopologyMode,
}

/// Compiled, immutable deployment plan for the active controller or local
/// roymctl runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentPlan {
    pub app_instance_id: AppInstanceId,
    pub blueprint_id: AppBlueprintId,
    pub version: semver::Version,
    #[serde(default)]
    pub services: Vec<PlannedService>,
}

impl DeploymentPlan {
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| anyhow!("Failed to parse TOML deployment plan: {}", e))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| anyhow!("Failed to parse JSON deployment plan: {}", e))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self)
            .map_err(|e| anyhow!("Failed to serialize to TOML deployment plan: {}", e))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize to JSON deployment plan: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_parsing_toml() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"
            description = "Professional Services Guild App"

            [services.identity]
            service_type = "wasm"
            source = "crates/app_sandbox/benches/identity.wasm"
            interfaces = ["syneroym:identity/identity"]
            depends_on = []

            [services.echo]
            service_type = "wasm"
            source = "crates/app_sandbox/benches/echo.wasm"
            interfaces = ["syneroym:echo/echo"]
            depends_on = ["identity"]

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
            manifest_path = "path/to/db.toml"
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        assert_eq!(manifest.id.as_str(), "syneroym:guild-app");
        assert_eq!(manifest.services.len(), 2);
        assert_eq!(manifest.dependencies.len(), 1);

        let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
        assert_eq!(identity.config.service_type, ServiceType::Wasm);
        assert_eq!(identity.config.source, "crates/app_sandbox/benches/identity.wasm");

        let db_dep = manifest.dependencies.get(&DependencyName::new("db")).unwrap();
        match db_dep {
            AppDependencySpec::Spawn { blueprint, manifest_path } => {
                assert_eq!(blueprint.as_str(), "syneroym:db-app");
                assert_eq!(manifest_path.as_deref(), Some("path/to/db.toml"));
            }
            _ => panic!("Expected Spawn dependency"),
        }

        // Test serialization roundtrip
        let serialized = manifest.to_toml().unwrap();
        let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
        assert_eq!(manifest, deserialized);
    }

    #[test]
    fn test_manifest_parsing_json() {
        let json_str = r#"{
            "id": "syneroym:guild-app",
            "version": "0.1.0",
            "description": "Professional Services Guild App",
            "services": {
                "identity": {
                    "service_type": "wasm",
                    "source": "crates/app_sandbox/benches/identity.wasm",
                    "interfaces": ["syneroym:identity/identity"],
                    "depends_on": []
                }
            },
            "dependencies": {
                "db": {
                    "mode": "bind",
                    "instance": "inst-1234"
                }
            }
        }"#;

        let manifest = SynAppManifest::from_json(json_str).unwrap();
        assert_eq!(manifest.id.as_str(), "syneroym:guild-app");
        assert_eq!(manifest.services.len(), 1);
        assert_eq!(manifest.dependencies.len(), 1);

        let db_dep = manifest.dependencies.get(&DependencyName::new("db")).unwrap();
        match db_dep {
            AppDependencySpec::Bind { instance } => {
                assert_eq!(instance.as_str(), "inst-1234");
            }
            _ => panic!("Expected Bind dependency"),
        }

        // Test serialization roundtrip
        let serialized = manifest.to_json().unwrap();
        let deserialized = SynAppManifest::from_json(&serialized).unwrap();
        assert_eq!(manifest, deserialized);
    }

    #[test]
    fn test_deployment_plan_serialization() {
        let mut env_map = BTreeMap::new();
        env_map.insert("KEY".to_string(), "VAL".to_string());

        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("guild-instance-1"),
            blueprint_id: AppBlueprintId::new("syneroym:guild-app"),
            version: semver::Version::parse("0.1.0").unwrap(),
            services: vec![PlannedService {
                service_id: ServiceId::new("did:key:h123"),
                logical_ref: LogicalServiceRef {
                    app_instance_id: AppInstanceId::new("guild-instance-1"),
                    service_name: LogicalServiceName::new("identity"),
                },
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "crates/app_sandbox/benches/identity.wasm".to_string(),
                    hash: None,
                    interfaces: vec![InterfaceName::new("syneroym:identity/identity")],
                    env: env_map,
                    args: vec![],
                    custom_config: None,
                },
                resolved_dependencies: vec![],
                topology_mode: TopologyMode::Singleton,
            }],
        };

        let toml_str = plan.to_toml().unwrap();
        let plan_toml = DeploymentPlan::from_toml(&toml_str).unwrap();
        assert_eq!(plan, plan_toml);

        let json_str = plan.to_json().unwrap();
        let plan_json = DeploymentPlan::from_json(&json_str).unwrap();
        assert_eq!(plan, plan_json);

        // Detailed field assertion test
        assert_eq!(plan_toml.app_instance_id, AppInstanceId::new("guild-instance-1"));
        assert_eq!(plan_toml.version.to_string(), "0.1.0");
        assert_eq!(plan_toml.services.len(), 1);
        let service = &plan_toml.services[0];
        assert_eq!(service.service_id, ServiceId::new("did:key:h123"));
        assert_eq!(service.topology_mode, TopologyMode::Singleton);
        assert_eq!(service.config.service_type, ServiceType::Wasm);
    }

    #[test]
    fn test_logical_service_ref_from_str() {
        let s = "guild-instance-1/identity";
        let r = LogicalServiceRef::from_str(s).unwrap();
        assert_eq!(r.app_instance_id, AppInstanceId::new("guild-instance-1"));
        assert_eq!(r.service_name, LogicalServiceName::new("identity"));
        assert_eq!(r.to_string(), s);

        assert!(LogicalServiceRef::from_str("invalid").is_err());
        assert!(LogicalServiceRef::from_str("too/many/parts").is_err());
    }

    #[test]
    fn test_id_validations() {
        assert!(LogicalServiceName::try_new("").is_err());
        assert!(LogicalServiceName::try_new("some/name").is_err());
        assert!(LogicalServiceName::try_new("good-name").is_ok());

        assert!(ServiceId::try_new("not-did-key").is_err());
        assert!(ServiceId::try_new("did:key:123").is_ok());
    }

    #[test]
    fn test_negative_parsing_and_validation() {
        // Missing required field
        let malformed_toml = r#"
            id = "syneroym:bad"
        "#;
        assert!(SynAppManifest::from_toml(malformed_toml).is_err());

        // Circular dependency
        let circular_toml = r#"
            id = "syneroym:bad"
            version = "0.1.0"
            [services.a]
            service_type = "wasm"
            source = "a"
            depends_on = ["b"]

            [services.b]
            service_type = "wasm"
            source = "b"
            depends_on = ["a"]
        "#;
        let manifest_res = SynAppManifest::from_toml(circular_toml);
        assert!(manifest_res.is_err());
        assert!(manifest_res.err().unwrap().to_string().contains("Circular dependency"));

        // Undefined dependency
        let missing_dep_toml = r#"
            id = "syneroym:bad"
            version = "0.1.0"
            [services.a]
            service_type = "wasm"
            source = "a"
            depends_on = ["nonexistent"]
        "#;
        let manifest_res2 = SynAppManifest::from_toml(missing_dep_toml);
        assert!(manifest_res2.is_err());
        assert!(manifest_res2.err().unwrap().to_string().contains("undefined service"));
    }

    #[test]
    fn test_toml_env_serialization() {
        let mut env_map = BTreeMap::new();
        env_map.insert("DATABASE_URL".to_string(), "postgres://...".to_string());
        env_map.insert("PORT".to_string(), "8080".to_string());

        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("guild-instance-1"),
            blueprint_id: AppBlueprintId::new("syneroym:guild-app"),
            version: semver::Version::parse("0.1.0").unwrap(),
            services: vec![PlannedService {
                service_id: ServiceId::new("did:key:h123"),
                logical_ref: LogicalServiceRef {
                    app_instance_id: AppInstanceId::new("guild-instance-1"),
                    service_name: LogicalServiceName::new("identity"),
                },
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "crates/app_sandbox/benches/identity.wasm".to_string(),
                    hash: None,
                    interfaces: vec![],
                    env: env_map,
                    args: vec![],
                    custom_config: None,
                },
                resolved_dependencies: vec![],
                topology_mode: TopologyMode::Singleton,
            }],
        };

        let toml_str = plan.to_toml().unwrap();
        assert!(toml_str.contains("DATABASE_URL"));
        assert!(toml_str.contains("PORT"));
    }
}
