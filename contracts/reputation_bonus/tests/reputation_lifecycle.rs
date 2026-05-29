#![cfg(test)]
use reputation_bonus::config::Config;
use reputation_bonus::{ReputationBonusContract, ReputationBonusContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn test_reputation_lifecycle_flow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(ReputationBonusContract, ());
    let client = ReputationBonusContractClient::new(&env, &contract_id);
    client.init(&admin);

    let config = Config {
        high_rep_threshold: 80,
        bonus_bps: 200,
        min_discount_rate_bps: 100,
    };
    client.set_config(&config);

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);

    // 1. Initial State
    let initial_rep = client.get_reputation(&freelancer);
    assert_eq!(initial_rep.score, 0);
    assert_eq!(initial_rep.invoices_submitted, 0);

    // 2. Submit Invoice
    let inv = client.submit_invoice(&freelancer, &payer, &1000, &1800000000, &500);
    let rep_f1 = client.get_reputation(&freelancer);
    let rep_p1 = client.get_reputation(&payer);
    
    assert_eq!(rep_f1.invoices_submitted, 1);
    assert_eq!(rep_p1.invoices_submitted, 1);
    assert_eq!(rep_f1.score, 0);

    // 3. Mark Paid
    client.mark_paid(&inv.id);
    let rep_f2 = client.get_reputation(&freelancer);
    let rep_p2 = client.get_reputation(&payer);
    
    assert_eq!(rep_f2.invoices_paid, 1);
    assert_eq!(rep_p2.invoices_paid, 1);
    assert_eq!(rep_f2.score, 100);
    assert_eq!(rep_p2.score, 100);

    // 4. Submit another and Default
    let inv2 = client.submit_invoice(&freelancer, &payer, &1000, &1800000000, &500);
    client.handle_default(&inv2.id);
    
    let rep_f3 = client.get_reputation(&freelancer);
    let rep_p3 = client.get_reputation(&payer);
    
    assert_eq!(rep_f3.invoices_defaulted, 1);
    assert_eq!(rep_p3.invoices_defaulted, 1);
    assert_eq!(rep_f3.invoices_submitted, 2);
    assert_eq!(rep_f3.score, 50); // (1/2)*100
}

#[test]
fn test_reputation_bonus_integration() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(ReputationBonusContract, ());
    let client = ReputationBonusContractClient::new(&env, &contract_id);
    client.init(&admin);

    let config = Config {
        high_rep_threshold: 80,
        bonus_bps: 200,
        min_discount_rate_bps: 100,
    };
    client.set_config(&config);

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);

    // 1. First invoice - 0 rep, no bonus
    let inv1 = client.submit_invoice(&freelancer, &payer, &1000, &1800000000, &500);
    assert_eq!(inv1.effective_discount_rate_bps, 500);

    // 2. Pay it - rep becomes 100
    client.mark_paid(&inv1.id);
    assert_eq!(client.get_reputation(&freelancer).score, 100);

    // 3. Next invoice - 100 rep, bonus applied
    let inv2 = client.submit_invoice(&freelancer, &payer, &1000, &1800000000, &500);
    // Effective rate = 500 - 200 = 300
    assert_eq!(inv2.effective_discount_rate_bps, 300);
}
