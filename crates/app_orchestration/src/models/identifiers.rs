use std::{error, fmt, ops::Deref, str::FromStr};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for ParseError {}

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

        impl Deref for $name {
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
define_string_wrapper!(
    AppInstanceId,
    "Unique identifier for a running application instance.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("AppInstanceId cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("AppInstanceId cannot contain '/'"));
        }
        // `#` is `MemberRef`'s own index separator. Forbidding it here, on
        // the instance-id half of the
        // boundary it splits on, is what closes the pre-existing
        // `member_master_name` collision between instance `a` + service
        // `b-c` and instance `a-b` + service `c` -- the last segment of a
        // valid `MemberRef` display form is always a bare `u32`, so only
        // the instance/service boundary needs the guard.
        if s.contains('#') {
            return Err(anyhow!("AppInstanceId cannot contain '#'"));
        }
        // `..` and `\` are valid here but not as a vault backup name
        // (`validate_backup_name`,
        // `crates/app_supervisor/src/keys.rs`) -- an instance id built
        // from one of these was refused only at `adopt`'s app-master mint,
        // by which point a services-less plan had already been accepted
        // by `submit`. Closed at construction instead: an id that can
        // never be backed up must never exist, not merely fail late and
        // permanently on the one verb (`adopt`) that is also the only way
        // back from `retired`.
        if s.contains("..") {
            return Err(anyhow!("AppInstanceId cannot contain '..'"));
        }
        if s.contains('\\') {
            return Err(anyhow!("AppInstanceId cannot contain '\\'"));
        }
        Ok(())
    }
);

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
        // `#` is `MemberRef`'s own index separator -- forbidden here so a
        // `MemberRef` display string can always be
        // parsed back by splitting on the last `#`, unambiguously.
        if s.contains('#') {
            return Err(anyhow!("LogicalServiceName cannot contain '#'"));
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

define_string_wrapper!(
    AppDid,
    "An app instance's own master DID (ADR-0022 §1) -- its network identity, as distinct from \
     `AppInstanceId`, its human name.",
    |s: &str| {
        if !s.starts_with("did:key:") {
            return Err(anyhow!("AppDid must start with 'did:key:'"));
        }
        // A validly-derived did:key never contains either character, so
        // this only ever refuses a malformed one -- but this value is
        // interpolated straight into a `synapp:<app-did>` `ResourceUri`
        // (ADR-0022 §5), where a stray `/` would produce a
        // selector-bearing resource `covers_resource` treats under a
        // different rule. Same two characters `AppInstanceId` and
        // `LogicalServiceName` already forbid.
        if s.contains('/') {
            return Err(anyhow!("AppDid cannot contain '/'"));
        }
        if s.contains('#') {
            return Err(anyhow!("AppDid cannot contain '#'"));
        }
        Ok(())
    }
);

define_string_wrapper!(InterfaceName, "Name of the interface a service implements.");
define_string_wrapper!(DependencyName, "Name of a dependency within an application.");

define_string_wrapper!(
    SubstrateAlias,
    "Operator-chosen name for a substrate in the deploy inventory.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("SubstrateAlias cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("SubstrateAlias cannot contain '/'"));
        }
        // Placement names an inventory alias, never a bare DID: an alias is
        // the indirection that lets one manifest deploy against different
        // operators' topologies, and a DID written here would defeat it.
        if s.starts_with("did:") {
            return Err(anyhow!(
                "SubstrateAlias '{s}' looks like a DID; placement names an inventory alias"
            ));
        }
        Ok(())
    }
);

/// How a service's hosting substrate is chosen.
///
/// One variant today. It is an enum rather than a bare alias so a later
/// pool- or constraint-based selector is an added variant instead of a
/// schema change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementSelector {
    /// Place on the substrate registered in the deploy inventory under this
    /// alias.
    Substrate(SubstrateAlias),
}

impl PlacementSelector {
    pub fn alias(&self) -> &SubstrateAlias {
        match self {
            Self::Substrate(alias) => alias,
        }
    }
}

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

/// Identifies one managed **member** of a logical service, as distinct from
/// the logical service itself.
///
/// `LogicalServiceRef` is the key of a logical service -- what the resolver,
/// `TopologyEntry`, and a binding's dependency name are about. `replicas`
/// makes that key stop being unique for anything the supervisor stores or
/// reports per *managed unit*: a placement row, a binding epoch, a
/// restart-attempt counter, an alert, a `revoke-instance` argument. Those
/// sites key on `MemberRef` instead; `LogicalServiceRef` is unchanged and
/// keeps its own meaning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MemberRef {
    pub logical_ref: LogicalServiceRef,
    pub index: u32,
}

impl fmt::Display for MemberRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.logical_ref, self.index)
    }
}

impl FromStr for MemberRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (l_ref_part, index_part) = s.rsplit_once('#').ok_or_else(|| {
            anyhow!("MemberRef must format as 'app_instance_id/service_name#index'")
        })?;
        let index = index_part
            .parse::<u32>()
            .map_err(|e| anyhow!("MemberRef index '{index_part}' is not a valid u32: {e}"))?;
        let logical_ref = LogicalServiceRef::from_str(l_ref_part)?;
        Ok(MemberRef { logical_ref, index })
    }
}

impl TryFrom<String> for MemberRef {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl From<MemberRef> for String {
    fn from(r: MemberRef) -> Self {
        r.to_string()
    }
}
