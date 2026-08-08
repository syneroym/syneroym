//! The saga compensation convention (ADR-0023 §7, as amended). A service
//! that can undo an operation exports a second function beside it, named by
//! prefixing the forward operation.

/// The reserved prefix a compensation is named with. Reserved is the
/// operative word: a plain `undo-` is an ordinary business verb
/// (`undo-last-update` is a perfectly good API), so a bare `undo-` marker
/// would make both the deploy check and the backward walk ambiguous -- the
/// walk could call a business function believing it to be a compensation.
/// Nothing in a domain interface begins with `saga-`.
///
/// One constant, read by the deploy gate and by the walk, so the two can
/// never spell it differently.
pub const SAGA_UNDO_PREFIX: &str = "saga-undo-";

/// The compensation's name for a forward operation.
#[must_use]
pub fn saga_undo_name(method: &str) -> String {
    format!("{SAGA_UNDO_PREFIX}{method}")
}

/// The forward operation a compensation names, or `None` when `function` is
/// not a compensation at all.
#[must_use]
pub fn compensated_operation(function: &str) -> Option<&str> {
    function.strip_prefix(SAGA_UNDO_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saga_undo_name_and_compensated_operation_round_trip() {
        let undo = saga_undo_name("reserve");
        assert_eq!(undo, "saga-undo-reserve");
        assert_eq!(compensated_operation(&undo), Some("reserve"));
    }

    #[test]
    fn compensated_operation_ignores_a_plain_undo_prefixed_function() {
        assert_eq!(compensated_operation("undo-last-update"), None);
        assert_eq!(compensated_operation("undo-reserve"), None);
    }

    #[test]
    fn compensated_operation_ignores_an_unrelated_function() {
        assert_eq!(compensated_operation("reserve"), None);
    }
}
