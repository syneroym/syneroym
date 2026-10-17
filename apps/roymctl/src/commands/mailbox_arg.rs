//! Command-line parsers for the mailbox options of `svc deploy` and
//! `app deploy`.

use syneroym_app_orchestration::MailboxChoice;

/// Parses `<service-name>=<mailbox>` for `app deploy --mailbox`. The mailbox
/// is `auto` or the mailbox host's node DID; `auto` is refused later, where
/// the plan is checked.
pub(crate) fn parse_service_mailbox(text: &str) -> Result<(String, MailboxChoice), String> {
    let (name, mailbox) = text
        .split_once('=')
        .ok_or_else(|| format!("'{text}' is not <service-name>=<mailbox-service-id>"))?;
    if name.trim().is_empty() {
        return Err(format!("'{text}' names no service before '='"));
    }
    let choice = mailbox.parse::<MailboxChoice>().map_err(|e| e.to_string())?;
    Ok((name.trim().to_string(), choice))
}

#[cfg(test)]
mod tests;
