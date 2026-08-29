//! The logical service names this app declares, and the WIT interface each
//! one answers on. The manifest's `interfaces` array, `proxy.call`'s
//! `interface` argument, and the native registration all read these -- one
//! definition, so a rename cannot land in two of the three.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    pub name: &'static str,
    pub interface: &'static str,
}

pub const WEB: Service = Service { name: "web", interface: "syneroym-roym:web/api@0.1.0" };
pub const CONVERSATION: Service =
    Service { name: "conversation", interface: "syneroym-roym:conversation/api@0.1.0" };
pub const PROFILE: Service =
    Service { name: "profile", interface: "syneroym-roym:profile/api@0.1.0" };
pub const CATALOG: Service =
    Service { name: "catalog", interface: "syneroym-roym:catalog/api@0.1.0" };
pub const TRANSACTION: Service =
    Service { name: "transaction", interface: "syneroym-roym:transaction/api@0.1.0" };
pub const DIRECTORY: Service =
    Service { name: "directory", interface: "syneroym-roym:directory/api@0.1.0" };

/// Every service, in manifest order. `web` first.
pub const ALL: [Service; 6] = [WEB, CONVERSATION, PROFILE, CATALOG, TRANSACTION, DIRECTORY];

/// The five `web` declares `depends_on` (everything but itself).
pub const SIBLINGS: [Service; 5] = [CONVERSATION, PROFILE, CATALOG, TRANSACTION, DIRECTORY];
