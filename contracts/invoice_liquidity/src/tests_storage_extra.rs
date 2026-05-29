#![cfg(test)]

// This file contains conservative storage-roundtrip checks for the compressed
// invoice and dispute/appeal record representations.

use super::*;
use soroban_sdk::BytesN;

#[test]
fn test_invoice_storage_roundtrip_u32_timestamps() {
    let t = setup();
    let invoice = Invoice {
        id: 42,
        freelancer: t.freelancer.clone(),
        payer: t.payer.clone(),
        token: t.token.address.clone(),
        amount: 1_000_000_000,
        due_date: 1_700_000_000u64.try_into().unwrap(),
        discount_rate: 300,
        status: InvoiceStatus::Pending,
        funder: Some(t.funder.clone()),
        funded_at: Some(1_700_000_100u64.try_into().unwrap()),
        amount_funded: 500_000_000,
        submitter_reputation: 55,
    };

    save_invoice(&t.env, &invoice);
    let loaded = load_invoice(&t.env, invoice.id);
    assert_eq!(loaded, invoice);
}

#[test]
fn test_appeal_and_dispute_record_storage_roundtrip() {
    let t = setup();
    let invoice_id = 99;

    let appeal = AppealRecord {
        evidence_hash: BytesN::from_array(&t.env, &[0xAA; 32]),
        appealed_at: 1_700_000_500u64.try_into().unwrap(),
        pre_default_score: 72,
    };
    save_appeal(&t.env, invoice_id, &appeal);
    assert_eq!(get_appeal(&t.env, invoice_id).unwrap(), appeal);

    let dispute = DisputeRecord {
        reason_hash: BytesN::from_array(&t.env, &[0xBB; 32]),
        disputed_at: 12345u32,
    };
    save_dispute(&t.env, invoice_id, &dispute);
    assert_eq!(get_dispute(&t.env, invoice_id).unwrap(), dispute);
}
