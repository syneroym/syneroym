#![allow(clippy::cognitive_complexity)]

pub(super) mod failing_storage;
pub(super) mod helpers;

mod app_instance;
mod assets;
mod cert;
mod deploy;
mod fdae_policy;
mod lifecycle;
mod mailbox;
mod probes;
mod proxy_queue;
mod rollback;
mod status;
