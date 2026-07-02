//! Vault WIT bindings
//!
//! Contains generated bindings allowing guest applications to access the vault.

wit_bindgen::generate!({
    world: "vault-guest",
    path: "wit/vault.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
