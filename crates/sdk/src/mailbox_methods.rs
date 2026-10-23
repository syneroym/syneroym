//! The check of `mailbox_methods` that needs the node's reserved names.

use syneroym_app_orchestration::DeploymentPlan;
use syneroym_core::mailbox::is_reserved_mailbox_interface;

/// The services of `plan` that list a node or platform interface in
/// `mailbox_methods`, as error texts. The substrate refuses them at deploy;
/// this refuses them before anything is placed.
pub fn refuse_reserved_mailbox_methods(plan: &DeploymentPlan) -> Result<(), Vec<String>> {
    let errors: Vec<String> = plan
        .services
        .iter()
        .flat_map(|service| {
            let name = &service.logical_ref.service_name;
            service
                .mailbox_methods
                .iter()
                .filter(|method| is_reserved_mailbox_interface(method.interface()))
                .map(move |method| {
                    format!(
                        "'{name}' lists mailbox method '{method}', which names a node or platform \
                         interface; only a guest's own methods can accept delivery from a mailbox"
                    )
                })
        })
        .collect();
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

#[cfg(test)]
mod tests;
