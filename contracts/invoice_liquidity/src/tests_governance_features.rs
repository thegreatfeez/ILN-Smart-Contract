#![cfg(test)]

//! Tests for the governance / reputation features:
//!  * #19 token allowlist enforcement in `fund_invoice`
//!  * #26 `ReputationProfile` struct + storage + getter
//!  * #28 minimum payer reputation threshold filtering
//!  * #71 hot-path single-read behaviour preserved

use super::*;
use crate::invoice::{get_reputation, set_reputation, ReputationProfile};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

const DUE_DATE_OFFSET: u64 = 60 * 60 * 24 * 30;
const DISCOUNT_RATE: u32 = 300;
const AMOUNT: i128 = 10_000_000;

struct MockToken {
    address: Address,
    #[allow(dead_code)]
    client: TokenClient<'static>,
    admin_client: StellarAssetClient<'static>,
}

struct TestEnv {
    env: Env,
    contract: InvoiceLiquidityContractClient<'static>,
    contract_id: Address,
    freelancer: Address,
    payer: Address,
    lp: Address,
    usdc: MockToken,
    xlm: MockToken,
}

fn register_mock_token(env: &Env) -> MockToken {
    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin);
    let token_address = token_contract.address();
    MockToken {
        address: token_address.clone(),
        client: TokenClient::new(env, &token_address),
        admin_client: StellarAssetClient::new(env, &token_address),
    }
}

fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let lp = Address::generate(&env);

    let usdc = register_mock_token(&env);
    let xlm = register_mock_token(&env);

    usdc.admin_client.mint(&payer, &100_000_000_000);
    usdc.admin_client.mint(&lp, &100_000_000_000);
    xlm.admin_client.mint(&lp, &100_000_000_000);

    let contract_id = env.register(InvoiceLiquidityContract, ());
    let contract = InvoiceLiquidityContractClient::new(&env, &contract_id);
    contract.initialize(&admin, &usdc.address, &xlm.address);

    let mut ledger_info = env.ledger().get();
    ledger_info.timestamp = 1_700_000_000;
    env.ledger().set(ledger_info);

    TestEnv {
        env,
        contract,
        contract_id,
        freelancer,
        payer,
        lp,
        usdc,
        xlm,
    }
}

fn submit(t: &TestEnv, token: &Address) -> u64 {
    let due = t.env.ledger().timestamp() + DUE_DATE_OFFSET;
    t.contract.submit_invoice(
        &t.freelancer,
        &t.payer,
        &AMOUNT,
        &due,
        &DISCOUNT_RATE,
        token,
    )
}

// ── Issue #19: token allowlist ───────────────────────────────────────────

#[test]
fn fund_succeeds_for_allowlisted_token() {
    let t = setup();
    let id = submit(&t, &t.usdc.address);
    // usdc was allowlisted at init → funding works.
    t.contract.fund_invoice(&t.lp, &id, &AMOUNT);
    assert_eq!(t.contract.get_invoice(&id).status, InvoiceStatus::Funded);
}

#[test]
fn fund_fails_after_token_removed_from_allowlist() {
    let t = setup();
    let id = submit(&t, &t.usdc.address);
    // Governance removes the token after submission.
    t.contract.remove_token(&t.usdc.address);
    let result = t.contract.try_fund_invoice(&t.lp, &id, &AMOUNT);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn add_token_then_fund_succeeds() {
    let t = setup();
    let new_token = register_mock_token(&t.env);
    new_token.admin_client.mint(&t.lp, &100_000_000_000);
    t.contract.add_token(&new_token.address);

    let id = submit(&t, &new_token.address);
    t.contract.fund_invoice(&t.lp, &id, &AMOUNT);
    assert_eq!(t.contract.get_invoice(&id).status, InvoiceStatus::Funded);
}

// ── Issue #26: ReputationProfile ─────────────────────────────────────────

#[test]
fn reputation_unknown_address_returns_zero_profile() {
    let t = setup();
    let who = Address::generate(&t.env);
    let profile = t.contract.get_reputation(&who);
    assert_eq!(profile.address, who);
    assert_eq!(profile.invoices_submitted, 0);
    assert_eq!(profile.invoices_paid, 0);
    assert_eq!(profile.invoices_defaulted, 0);
    assert_eq!(profile.score, 0);
}

#[test]
fn reputation_fields_update_correctly() {
    let t = setup();
    let who = Address::generate(&t.env);
    let updated = ReputationProfile {
        address: who.clone(),
        invoices_submitted: 4,
        invoices_paid: 3,
        invoices_defaulted: 1,
        score: 72,
    };
    // Exercise the storage helpers directly within the contract context.
    t.env.as_contract(&t.contract_id, || {
        set_reputation(&t.env, &updated);
        let read = get_reputation(&t.env, &who);
        assert_eq!(read, updated);
    });
    // And via the public view.
    let via_view = t.contract.get_reputation(&who);
    assert_eq!(via_view.invoices_submitted, 4);
    assert_eq!(via_view.invoices_paid, 3);
    assert_eq!(via_view.invoices_defaulted, 1);
    assert_eq!(via_view.score, 72);
}

// ── Issue #28: reputation threshold ──────────────────────────────────────

#[test]
fn threshold_defaults_to_zero_and_is_updatable() {
    let t = setup();
    assert_eq!(t.contract.min_payer_reputation(), 0);
    t.contract.set_min_payer_reputation(&25);
    assert_eq!(t.contract.min_payer_reputation(), 25);
}

#[test]
fn fund_succeeds_when_payer_reputation_meets_threshold() {
    let t = setup();
    // Fresh payers have the neutral default score of 50.
    t.contract.set_min_payer_reputation(&40);
    let id = submit(&t, &t.usdc.address);
    t.contract.fund_invoice(&t.lp, &id, &AMOUNT);
    assert_eq!(t.contract.get_invoice(&id).status, InvoiceStatus::Funded);
}

#[test]
fn fund_fails_when_payer_reputation_below_threshold() {
    let t = setup();
    t.contract.set_min_payer_reputation(&60); // above the default 50
    let id = submit(&t, &t.usdc.address);
    let result = t.contract.try_fund_invoice(&t.lp, &id, &AMOUNT);
    assert_eq!(result, Err(Ok(ContractError::PayerReputationTooLow)));
}

// ── Issue #71: hot-path behaviour unchanged ──────────────────────────────

#[test]
fn fund_nonexistent_invoice_returns_not_found() {
    let t = setup();
    let result = t.contract.try_fund_invoice(&t.lp, &999, &AMOUNT);
    assert_eq!(result, Err(Ok(ContractError::InvoiceNotFound)));
}

#[test]
fn mark_paid_nonexistent_invoice_returns_not_found() {
    let t = setup();
    let result = t.contract.try_mark_paid(&999, &AMOUNT);
    assert_eq!(result, Err(Ok(ContractError::InvoiceNotFound)));
}

#[test]
fn fund_then_mark_paid_full_lifecycle_still_works() {
    let t = setup();
    let id = submit(&t, &t.usdc.address);
    t.contract.fund_invoice(&t.lp, &id, &AMOUNT);
    t.contract.mark_paid(&id, &AMOUNT);
    assert_eq!(t.contract.get_invoice(&id).status, InvoiceStatus::Paid);
}
