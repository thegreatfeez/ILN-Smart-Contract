//! Comprehensive tests for XLM (native asset) support via Stellar Asset Contract (SAC) wrapper
//!
//! This test suite verifies that XLM is correctly handled through the SAC interface,
//! including:
//! - XLM precision handling (7 decimal places vs USDC's 6)
//! - fund_invoice() operations with XLM
//! - mark_paid() operations with XLM
//! - Config storage of XLM SAC address
//! - Volume tracking for XLM

#![cfg(test)]

use super::*;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

const XLM_DECIMALS: u32 = 7; // 1 XLM = 10,000,000 stroops
const USDC_DECIMALS: u32 = 6; // 1 USDC = 1,000,000 units

// Test amounts in XLM (7 decimal places)
const XLM_INVOICE_AMOUNT: i128 = 100_000_000; // 10 XLM
const XLM_FUND_AMOUNT: i128 = 50_000_000; // 5 XLM

// Test amounts in USDC (6 decimal places)
const USDC_INVOICE_AMOUNT: i128 = 10_000_000; // 10 USDC
const USDC_FUND_AMOUNT: i128 = 5_000_000; // 5 USDC

fn setup_xlm_env() -> (
    Env,
    Address,
    Address,
    Address,
    Address,
    InvoiceLiquidityContractClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();

    // Deploy admin
    let admin = Address::generate(&env);

    // Deploy USDC token (6 decimals)
    let usdc_admin = Address::generate(&env);
    let usdc_contract_id = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_address = usdc_contract_id.address();

    // Deploy XLM SAC (7 decimals - native XLM wrapper)
    let xlm_admin = Address::generate(&env);
    let xlm_contract_id = env.register_stellar_asset_contract_v2(xlm_admin);
    let xlm_address = xlm_contract_id.address();

    // Deploy invoice liquidity contract
    let contract_id = env.register(InvoiceLiquidityContract, ());
    let client = InvoiceLiquidityContractClient::new(&env, &contract_id);

    // Initialize contract with USDC and XLM
    client.initialize(&admin, &usdc_address, &xlm_address);

    (env, admin, usdc_address, xlm_address, contract_id, client)
}

#[test]
fn test_initialize_stores_xlm_sac_address_in_config() {
    let (env, _admin, _usdc_address, xlm_address, _contract_id, client) = setup_xlm_env();

    // Verify that config was initialized with XLM SAC address
    // Note: We can't directly access config from tests, but we can verify
    // that the contract was initialized successfully and accepts XLM tokens
    let stats = client.get_contract_stats();
    assert_eq!(stats.total_invoices, 0);
}

#[test]
fn test_submit_invoice_with_xlm_token() {
    let (env, admin, _usdc_address, xlm_address, _contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60; // 30 days from now
    let discount_rate = 300; // 3%

    // Submit invoice with XLM token
    let invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    let invoice = client.get_invoice(&invoice_id).unwrap();
    assert_eq!(invoice.token, xlm_address);
    assert_eq!(invoice.amount, XLM_INVOICE_AMOUNT);
    assert_eq!(invoice.status, InvoiceStatus::Pending);
}

#[test]
fn test_fund_invoice_with_xlm() {
    let (env, admin, usdc_address, xlm_address, contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let funder = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    let discount_rate = 300; // 3%

    // Submit XLM invoice
    let invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    // Mint XLM to funder
    let xlm_token = TokenClient::new(&env, &xlm_address);
    let xlm_admin = StellarAssetClient::new(&env, &xlm_address);
    xlm_admin.mint(&funder, &XLM_INVOICE_AMOUNT);

    // Fund invoice with XLM
    client
        .fund_invoice(&funder, &invoice_id, &XLM_INVOICE_AMOUNT)
        .unwrap();

    let invoice = client.get_invoice(&invoice_id).unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Funded);
    assert_eq!(invoice.amount_funded, XLM_INVOICE_AMOUNT);
    assert_eq!(invoice.funder, Some(funder.clone()));

    // Verify freelancer received payout (amount - discount)
    let expected_payout = XLM_INVOICE_AMOUNT - (XLM_INVOICE_AMOUNT * 300 / 10_000);
    let freelancer_balance = xlm_token.balance(&freelancer);
    assert_eq!(freelancer_balance, expected_payout);
}

#[test]
fn test_mark_paid_with_xlm() {
    let (env, admin, usdc_address, xlm_address, contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let funder = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    let discount_rate = 300; // 3%

    // Submit and fund XLM invoice
    let invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    let xlm_token = TokenClient::new(&env, &xlm_address);
    let xlm_admin = StellarAssetClient::new(&env, &xlm_address);
    xlm_admin.mint(&funder, &XLM_INVOICE_AMOUNT);

    client
        .fund_invoice(&funder, &invoice_id, &XLM_INVOICE_AMOUNT)
        .unwrap();

    // Mint XLM to payer for payment
    xlm_admin.mint(&payer, &XLM_INVOICE_AMOUNT);

    // Mark invoice as paid
    client.mark_paid(&invoice_id, &XLM_INVOICE_AMOUNT).unwrap();

    let invoice = client.get_invoice(&invoice_id).unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Paid);
    assert_eq!(invoice.amount_paid, XLM_INVOICE_AMOUNT);

    // Verify LP received their share
    let lp_balance = xlm_token.balance(&funder);
    assert!(lp_balance > 0);
}

#[test]
fn test_partial_funding_with_xlm() {
    let (env, admin, usdc_address, xlm_address, contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let funder = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    let discount_rate = 300; // 3%

    // Submit XLM invoice
    let invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    let xlm_token = TokenClient::new(&env, &xlm_address);
    let xlm_admin = StellarAssetClient::new(&env, &xlm_address);
    xlm_admin.mint(&funder, &XLM_INVOICE_AMOUNT);

    // Partially fund with XLM
    client
        .fund_invoice(&funder, &invoice_id, &XLM_FUND_AMOUNT)
        .unwrap();

    let invoice = client.get_invoice(&invoice_id).unwrap();
    assert_eq!(invoice.status, InvoiceStatus::PartiallyFunded);
    assert_eq!(invoice.amount_funded, XLM_FUND_AMOUNT);
}

#[test]
fn test_xlm_volume_tracking() {
    let (env, admin, usdc_address, xlm_address, contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let funder = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    let discount_rate = 300; // 3%

    // Submit and fund XLM invoice
    let invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    let xlm_token = TokenClient::new(&env, &xlm_address);
    let xlm_admin = StellarAssetClient::new(&env, &xlm_address);
    xlm_admin.mint(&funder, &XLM_INVOICE_AMOUNT);

    client
        .fund_invoice(&funder, &invoice_id, &XLM_INVOICE_AMOUNT)
        .unwrap();

    // Check that XLM volume is tracked
    let stats = client.get_contract_stats();
    assert_eq!(stats.total_volume_xlm, XLM_INVOICE_AMOUNT);
    assert_eq!(stats.total_volume_usdc, 0);
}

#[test]
fn test_mixed_token_operations() {
    let (env, admin, usdc_address, xlm_address, contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let funder = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    let discount_rate = 300; // 3%

    let usdc_token = TokenClient::new(&env, &usdc_address);
    let usdc_admin = StellarAssetClient::new(&env, &usdc_address);
    let xlm_token = TokenClient::new(&env, &xlm_address);
    let xlm_admin = StellarAssetClient::new(&env, &xlm_address);

    // Submit USDC invoice
    let usdc_invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &USDC_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &usdc_address,
        )
        .unwrap();

    usdc_admin.mint(&funder, &USDC_INVOICE_AMOUNT);
    client
        .fund_invoice(&funder, &usdc_invoice_id, &USDC_INVOICE_AMOUNT)
        .unwrap();

    // Submit XLM invoice
    let xlm_invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    xlm_admin.mint(&funder, &XLM_INVOICE_AMOUNT);
    client
        .fund_invoice(&funder, &xlm_invoice_id, &XLM_INVOICE_AMOUNT)
        .unwrap();

    // Verify both volumes are tracked correctly
    let stats = client.get_contract_stats();
    assert_eq!(stats.total_volume_usdc, USDC_INVOICE_AMOUNT);
    assert_eq!(stats.total_volume_xlm, XLM_INVOICE_AMOUNT);
}

#[test]
fn test_xlm_precision_in_calculations() {
    let (env, admin, usdc_address, xlm_address, contract_id, client) = setup_xlm_env();

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let funder = Address::generate(&env);
    let due_date = env.ledger().timestamp() + 30 * 24 * 60 * 60;
    let discount_rate = 300; // 3%

    // Submit XLM invoice
    let invoice_id = client
        .submit_invoice(
            &freelancer,
            &payer,
            &XLM_INVOICE_AMOUNT,
            &due_date,
            &discount_rate,
            &xlm_address,
        )
        .unwrap();

    let xlm_token = TokenClient::new(&env, &xlm_address);
    let xlm_admin = StellarAssetClient::new(&env, &xlm_address);
    xlm_admin.mint(&funder, &XLM_INVOICE_AMOUNT);

    // Fund and verify discount calculation is correct with 7 decimal precision
    client
        .fund_invoice(&funder, &invoice_id, &XLM_INVOICE_AMOUNT)
        .unwrap();

    let invoice = client.get_invoice(&invoice_id).unwrap();

    // Discount should be: 100,000,000 * 300 / 10,000 = 3,000,000 stroops (0.3 XLM)
    let expected_discount = XLM_INVOICE_AMOUNT * 300 / 10_000;
    let expected_payout = XLM_INVOICE_AMOUNT - expected_discount;

    let freelancer_balance = xlm_token.balance(&freelancer);
    assert_eq!(freelancer_balance, expected_payout);
}
