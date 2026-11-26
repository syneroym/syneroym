use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::{Commitment, CommitmentKind, LocalText},
    disclosure::decode_disclosure,
    eventlog::{DeriveInput, Derived, FormedFact, HeldSnapshot, SettlingHalf, derive},
    progress::CommitmentProgress,
    record::Envelope,
    transaction::DealRole,
};

use crate::app::{
    AGREEMENT_PROGRESS, AgreementRow, FulfilmentsRow, PaymentsRow, decide::rows::SnapshotRow,
    disclosures, fulfilment_ops::fulfilment_id, get_row, payment_ops::payment_id,
};

pub(crate) struct SettlingItem {
    pub(crate) commitment_id: String,
    pub(crate) records: Vec<String>,
    pub(crate) settling_issued_at_secs: Option<u64>,
    pub(crate) settlement_note: Option<LocalText>,
}

pub(crate) async fn gather_and_derive<H: AppHost>(
    host: &H,
    owner_did: &str,
    row: &AgreementRow,
) -> Result<Derived, String> {
    let owner_role = if owner_did == row.quote_author_did {
        DealRole::QuoteAuthor
    } else {
        DealRole::Counterparty
    };

    let counterparty_did = if owner_role == DealRole::QuoteAuthor {
        &row.counterparty_did
    } else {
        &row.quote_author_did
    };

    let (receipt_halves, quote_author_issued_at_secs) = extract_receipt_halves(row);
    let agreed_fact = (row.quote_author.is_some() || row.counterparty.is_some())
        .then_some(FormedFact { quote_author_issued_at_secs, receipt_halves: &receipt_halves });

    let snapshot_rows = load_snapshots(host, &row.quote_record_id, row.seq.unwrap_or(0)).await;
    let held_snapshots: Vec<HeldSnapshot<'_>> = snapshot_rows
        .iter()
        .map(|sr| {
            let issued_at_secs = Envelope::from_json(&sr.envelope)
                .map(|e| e.issued_at_secs)
                .unwrap_or(sr.stored_at_secs);
            HeldSnapshot {
                seq: sr.seq,
                issued_at_secs,
                record_id: &sr.record_id,
                state: sr.snapshot.state,
                commitments: &sr.snapshot.commitments,
            }
        })
        .collect();

    let settling_items = load_settling_items(host, row).await;
    let settling_halves: Vec<SettlingHalf<'_>> = settling_items
        .iter()
        .map(|s| SettlingHalf {
            commitment_id: &s.commitment_id,
            records: &s.records,
            settling_issued_at_secs: s.settling_issued_at_secs,
            settlement_note: s.settlement_note.as_ref(),
        })
        .collect();

    let input = DeriveInput {
        owner_role,
        agreement: &row.quote_record_id,
        conversation: &row.conversation,
        counterparty_did,
        terms: &row.terms,
        state: row.state,
        commitments: &row.commitments,
        agreed: agreed_fact,
        snapshots: &held_snapshots,
        settling: &settling_halves,
    };

    Ok(derive(&input))
}

fn extract_receipt_halves(row: &AgreementRow) -> (Vec<String>, Option<u64>) {
    let mut halves = Vec::new();
    if let Some(ref cp) = row.counterparty {
        halves.push(cp.record_id.clone());
    }
    if let Some(ref qa) = row.quote_author {
        halves.push(qa.record_id.clone());
    }
    let issued_at = row.quote_author.as_ref().map(|qa| qa.issued_at_secs);
    (halves, issued_at)
}

async fn load_snapshots<H: AppHost>(
    host: &H,
    agreement: &str,
    highest_seq: u32,
) -> Vec<SnapshotRow> {
    let mut rows = Vec::new();
    for seq in 1..=highest_seq {
        let key = crate::app::decide::rows::snapshot_id(agreement, seq);
        if let Ok(Some(sr)) = get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &key).await {
            rows.push(sr);
        }
    }
    rows
}

async fn load_settling_items<H: AppHost>(host: &H, row: &AgreementRow) -> Vec<SettlingItem> {
    let mut items = Vec::new();
    for comm in &row.terms.commitments {
        let receiver_role = match comm.giver {
            DealRole::QuoteAuthor => DealRole::Counterparty,
            DealRole::Counterparty => DealRole::QuoteAuthor,
        };

        let progress = row.commitments.iter().find(|c| c.commitment_id == comm.commitment_id);

        let item = match comm.kind {
            CommitmentKind::Currency => {
                load_currency_settling(host, row, comm, receiver_role, progress).await
            }
            CommitmentKind::Fulfilment => {
                load_fulfilment_settling(host, row, comm, receiver_role).await
            }
        };

        if let Some(item) = item {
            items.push(item);
        }
    }
    items
}

async fn load_currency_settling<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    comm: &Commitment,
    receiver_role: DealRole,
    progress: Option<&CommitmentProgress>,
) -> Option<SettlingItem> {
    let p_id = payment_id(&row.quote_record_id, &comm.commitment_id);
    let payments: PaymentsRow = get_row(host, crate::app::PAYMENTS, &p_id).await.ok()??;

    let mut records = Vec::new();
    if let Some(ref req) = payments.request {
        records.push(req.record_id.clone());
    }
    for cp in &payments.counterparty {
        records.push(cp.record_id.clone());
    }
    for qa in &payments.quote_author {
        records.push(qa.record_id.clone());
    }

    let receiver_halves = match receiver_role {
        DealRole::Counterparty => &payments.counterparty,
        DealRole::QuoteAuthor => &payments.quote_author,
    };

    let settling_half = if let Some(prog) = progress {
        receiver_halves
            .iter()
            .find(|h| prog.halves.contains(&h.record_id))
            .or_else(|| receiver_halves.first())
    } else {
        receiver_halves.first()
    };

    let settling_issued_at_secs = settling_half.map(|h| h.issued_at_secs);
    let settlement_note = if let Some(half) = settling_half {
        load_note_from_disclosures(host, &half.record_id).await
    } else {
        None
    };

    Some(SettlingItem {
        commitment_id: comm.commitment_id.clone(),
        records,
        settling_issued_at_secs,
        settlement_note,
    })
}

async fn load_fulfilment_settling<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    comm: &Commitment,
    receiver_role: DealRole,
) -> Option<SettlingItem> {
    let f_id = fulfilment_id(&row.quote_record_id, &comm.commitment_id);
    let fulfilments: FulfilmentsRow =
        get_row(host, crate::app::FULFILMENTS, &f_id).await.ok()??;

    let mut records = Vec::new();
    if let Some(ref cp) = fulfilments.counterparty {
        records.push(cp.record_id.clone());
    }
    if let Some(ref qa) = fulfilments.quote_author {
        records.push(qa.record_id.clone());
    }

    let settling_half = match receiver_role {
        DealRole::Counterparty => fulfilments.counterparty.as_ref(),
        DealRole::QuoteAuthor => fulfilments.quote_author.as_ref(),
    };

    let settling_issued_at_secs = settling_half.map(|h| h.issued_at_secs);
    let settlement_note = if let Some(half) = settling_half {
        load_note_from_disclosures(host, &half.record_id).await
    } else {
        None
    };

    Some(SettlingItem {
        commitment_id: comm.commitment_id.clone(),
        records,
        settling_issued_at_secs,
        settlement_note,
    })
}

async fn load_note_from_disclosures<H: AppHost>(host: &H, record_id: &str) -> Option<LocalText> {
    let disc_list = disclosures::load(host, record_id).await.ok()??;
    for d in disc_list {
        if let Ok(decoded) = decode_disclosure(&d)
            && decoded.name == "settlement_note"
            && let Ok(note) = serde_json::from_value::<LocalText>(decoded.value)
        {
            return Some(note);
        }
    }
    None
}
