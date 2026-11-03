//! `roymctl roym directory find`: fan out a search across every added
//! directory via one `directory.find` call, and print the merged, verified
//! result.

use std::io::{self, Write};

use anyhow::Result;
use serde_json::{Value, json};
use syneroym_roym_core::membership;

use super::parse_near;
use crate::commands::session;

/// The same wording the Hub shows for each `SourceError` kind.
pub(crate) fn source_error_words(kind: &str) -> &'static str {
    match kind {
        "not-started" => "this installation was busy and did not start the call",
        "timed-out" => "the directory did not answer in time",
        "not-found" => "no directory answers at that address",
        "refused" => "the directory refused the request",
        _ => "the directory's answer could not be read",
    }
}

pub(crate) fn source_note(outcome: &Value) -> Option<String> {
    let err = outcome.get("error");
    if let Some(err) = err
        && !err.is_null()
    {
        let kind = err.get("kind").and_then(Value::as_str).unwrap_or("unreadable");
        return Some(source_error_words(kind).to_string());
    }
    if outcome.get("truncated").and_then(Value::as_bool).unwrap_or(false) {
        return Some("this directory had more matches than it would return".to_string());
    }
    None
}

/// Words for one `sources[]` entry's `membership` value -- this node's
/// own verdict, computed from the signed evidence that source served,
/// never the source's own claim. `None` (a refused hit never reaches a
/// membership check) reads as "not checked".
pub(crate) fn membership_words(membership: Option<&Value>) -> String {
    let Some(v) = membership else { return "not checked".to_string() };
    let Ok(verdict) = serde_json::from_value::<membership::MembershipVerdict>(v.clone()) else {
        return "unreadable".to_string();
    };
    let word = membership::verdict_word(&verdict);
    match &verdict {
        membership::MembershipVerdict::Suspended { rule, reason, .. } => {
            format!("{word} (rule: {rule}, reason: {reason})")
        }
        membership::MembershipVerdict::Revoked { reason, .. } => format!("{word} ({reason})"),
        membership::MembershipVerdict::OutOfScope { outside, .. } => {
            format!("{word} ({})", outside.join(", "))
        }
        membership::MembershipVerdict::Unknown { reason }
        | membership::MembershipVerdict::Refused { reason } => format!("{word} ({reason})"),
        _ => word.to_string(),
    }
}

pub(crate) fn build_find_query(
    text: Option<&str>,
    categories: &[String],
    near: Option<&str>,
    limit: u32,
) -> Result<Value> {
    let mut query = json!({ "categories": categories, "limit": limit });
    if let Some(t) = text {
        query["text"] = json!(t);
    }
    if let Some(n) = near {
        query["area"] = parse_near(n)?;
    }
    Ok(query)
}

pub(crate) fn print_find_reply<W: Write>(mut out: W, reply: &Value) -> Result<()> {
    let empty_vec = vec![];
    let outcomes = reply.get("outcomes").and_then(|v| v.as_array()).unwrap_or(&empty_vec);

    if outcomes.is_empty() {
        writeln!(out, "No directories added. Add one with `roymctl roym directory add <did>`,")?;
        writeln!(out, "or reach a listing owner directly by link -- a directory is optional.")?;
    }

    for outcome in outcomes {
        let source = outcome.get("source").and_then(|v| v.as_str()).unwrap_or("?");
        if let Some(note) = source_note(outcome) {
            writeln!(out, "source {source}: {note}")?;
        }
    }

    let hits = reply.get("hits").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    writeln!(out, "{} result(s):", hits.len())?;
    for hit in hits {
        let listing_id = hit.get("listing_id").and_then(|v| v.as_str()).unwrap_or("?");
        let title = hit.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let issuer = hit.get("issuer").and_then(|v| v.as_str()).unwrap_or("?");
        let age = hit.get("age_secs").and_then(|v| v.as_u64()).unwrap_or(0);
        let revocation = hit.get("revocation_status").and_then(|v| v.as_str()).unwrap_or("unknown");
        writeln!(
            out,
            "- {listing_id} \"{title}\" by {issuer}, age {age}s, revocation: {revocation}"
        )?;
        let empty_sources = vec![];
        for source in hit.get("sources").and_then(|v| v.as_array()).unwrap_or(&empty_sources) {
            let directory = source.get("directory").and_then(|v| v.as_str()).unwrap_or("?");
            let words = membership_words(source.get("membership"));
            writeln!(out, "  source {directory}: membership {words}")?;
        }
    }

    let refused = reply.get("refused").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    if !refused.is_empty() {
        writeln!(
            out,
            "\n{} refused (never trusted, shown so you know a directory served them):",
            refused.len()
        )?;
        for r in refused {
            writeln!(out, "- {}", serde_json::to_string(r)?)?;
        }
    }
    Ok(())
}

pub(super) struct FindArgs<'a> {
    pub(super) text: Option<&'a str>,
    pub(super) categories: &'a [String],
    pub(super) near: Option<&'a str>,
    pub(super) limit: u32,
}

pub(super) async fn find(
    args: FindArgs<'_>,
    gateway_url: &str,
    host: Option<&str>,
    ctx: super::directory::RpcCtx<'_>,
) -> Result<()> {
    let query = build_find_query(args.text, args.categories, args.near, args.limit)?;
    let result = session::rpc_call(
        gateway_url,
        host,
        ctx.run_as,
        ctx.ucan_path,
        ctx.dir,
        "directory.find",
        json!({ "query": query }),
    )
    .await;

    handle_find_reply(io::stdout(), result)
}

fn handle_find_reply<W: Write>(writer: W, result: Result<Value>) -> Result<()> {
    let reply = result?;
    print_find_reply(writer, &reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canned_reply_failed_source_note() {
        let reply = json!({
            "outcomes": [
                {
                    "source": "did:key:s1",
                    "verified": 0,
                    "refused": 0,
                    "error": { "kind": "timed-out" }
                }
            ],
            "hits": [],
            "refused": []
        });
        let mut out = Vec::new();
        print_find_reply(&mut out, &reply).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("source did:key:s1: the directory did not answer in time"));
        assert!(s.contains("0 result(s):"));
    }

    #[test]
    fn canned_reply_truncated_note() {
        let reply = json!({
            "outcomes": [
                {
                    "source": "did:key:s2",
                    "verified": 1,
                    "refused": 0,
                    "truncated": true,
                    "error": null
                }
            ],
            "hits": [],
            "refused": []
        });
        let mut out = Vec::new();
        print_find_reply(&mut out, &reply).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("source did:key:s2: this directory had more matches than it would return")
        );
    }

    #[test]
    fn canned_reply_membership_lines() {
        let reply = json!({
            "outcomes": [],
            "hits": [
                {
                    "listing_id": "lst_1",
                    "title": "Honey",
                    "issuer": "did:key:zIss",
                    "age_secs": 5,
                    "revocation_status": "none",
                    "sources": [
                        {
                            "directory": "did:key:s1",
                            "membership": {
                                "state": "valid",
                                "credential_record_id": "rec_c",
                                "issuer": "did:key:zIss",
                                "synorg_name": "Guild",
                                "scope": { "categories": [], "area": [] },
                                "expires_at_secs": 100,
                                "revocations_checked_as_of_secs": 10
                            }
                        }
                    ]
                }
            ],
            "refused": []
        });
        let mut out = Vec::new();
        print_find_reply(&mut out, &reply).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("- lst_1 \"Honey\" by did:key:zIss, age 5s, revocation: none"));
        assert!(s.contains("source did:key:s1: membership valid"));
    }

    #[test]
    fn canned_reply_refused_block() {
        let reply = json!({
            "outcomes": [],
            "hits": [],
            "refused": [
                {
                    "listing_id": "lst_bad",
                    "reason": "signature did not verify",
                    "sources": ["did:key:s1"]
                }
            ]
        });
        let mut out = Vec::new();
        print_find_reply(&mut out, &reply).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("1 refused (never trusted, shown so you know a directory served them):")
        );
        assert!(s.contains("lst_bad"));
    }

    #[test]
    fn canned_reply_all_not_started() {
        let reply = json!({
            "outcomes": [
                { "source": "did:key:s1", "error": { "kind": "not-started" } },
                { "source": "did:key:s2", "error": { "kind": "not-started" } }
            ],
            "hits": [],
            "refused": []
        });
        let mut out = Vec::new();
        print_find_reply(&mut out, &reply).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("source did:key:s1: this installation was busy and did not start the call")
        );
        assert!(
            s.contains("source did:key:s2: this installation was busy and did not start the call")
        );
    }

    #[test]
    fn error_result_exits_non_zero() {
        let err: Result<Value> = Err(anyhow::anyhow!("connection refused"));
        let handled = handle_find_reply(&mut Vec::new(), err);
        assert!(handled.is_err());
        assert_eq!(handled.unwrap_err().to_string(), "connection refused");
    }
}
