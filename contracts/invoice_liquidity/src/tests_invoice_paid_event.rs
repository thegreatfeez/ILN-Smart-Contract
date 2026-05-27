use crate::{
    InvoiceLiquidityContract,
    InvoiceLiquidityContractClient,
};

use soroban_sdk::{
    token::StellarAssetClient,
    Address, Env,
};

#[test]
fn emits_invoice_paid_event_with_full_settlement_details() {
    use soroban_sdk::{
        testutils::{Address as _, Events},
        token::{Client as TokenClient, StellarAssetClient},
        Address, Env,
    };

    let env = Env::default();
    env.mock_all_auths();

    // ------------------------------------------------------------
    // Accounts
    // ------------------------------------------------------------
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);
    let lp = Address::generate(&env);

    // ------------------------------------------------------------
    // Token setup
    // ------------------------------------------------------------
    let token_admin = Address::generate(&env);

    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    // let token_client = TokenClient::new(&env, &token);
    let asset_client = StellarAssetClient::new(&env, &token);

    // Mint liquidity to LP + payer
    asset_client.mint(&lp, &10_000_000);
    asset_client.mint(&payer, &10_000_000);

    // ------------------------------------------------------------
    // Contract setup
    // ------------------------------------------------------------
    let contract_id = env.register(InvoiceLiquidityContract, ());
    let client = InvoiceLiquidityContractClient::new(&env, &contract_id);

    client.initialize(&admin, &token, &token);

    // ------------------------------------------------------------
    // Submit invoice
    // ------------------------------------------------------------
    let due_date = env.ledger().timestamp() + (7 * 24 * 60 * 60);

    let invoice_id = client.submit_invoice(
        &freelancer,
        &payer,
        &1_000_000_i128,
        &due_date,
        &1000_u32, // 10%
        &token,
    );

    // ------------------------------------------------------------
    // Fund invoice
    // ------------------------------------------------------------
    client.fund_invoice(
        &lp,
        &invoice_id,
        &1_000_000_i128,
    );

    // ------------------------------------------------------------
    // Mark paid
    // ------------------------------------------------------------
    client.mark_paid(&invoice_id);

    // ------------------------------------------------------------
    // Validate emitted event
    // ------------------------------------------------------------
    let events = env.events().all();

    let paid_event = events.last().unwrap();

    // ------------------------------------------------------------
    // Expected math
    // ------------------------------------------------------------
    let amount_paid: i128 = 1_000_000;

    // no protocol fee by default
    let lp_payout: i128 = 1_000_000;

    // payout - funded
    let lp_earned: i128 = 0;

    // ------------------------------------------------------------
    // Decode + assert event data
    // ------------------------------------------------------------
    let expected_event = InvoicePaid {
        invoice_id,
        payer: payer.clone(),
        lp: lp.clone(),
        freelancer: freelancer.clone(),
        token: token.clone(),
        amount_paid,
        lp_earned,
        lp_payout,
        settlement_timestamp: env.ledger().timestamp(),
        paid_on_time: true,
        status: InvoiceStatus::Paid,
    };

    assert_eq!(
        paid_event.1,
        expected_event.into_val(&env)
    );
}