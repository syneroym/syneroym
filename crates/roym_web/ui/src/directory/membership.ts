// A SynOrg's membership verdict, mirrored from `roym_core::membership`
// (Rust is the source of truth; a Rust-side test reads this file and
// compares the two notice strings verbatim).

export const NO_INSTANT_REMOVAL_NOTICE =
  "A group's decision reaches copies other people already hold only when they next check. Nobody can promise it is removed everywhere at once.";

export const WITHHELD_REVOCATION_NOTICE =
  "This shows every withdrawal the group has published. It cannot show one the group chose not to publish.";
