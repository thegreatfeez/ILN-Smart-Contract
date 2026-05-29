#![cfg(test)]

use reputation_bonus::config::Config;
use reputation_bonus::events::ParameterUpdated;
use reputation_bonus::rate_logic::calculate_effective_rate;
use reputation_bonus::{ReputationBonusContract, ReputationBonusContractClient};
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, Env, Event, Symbol,
};

#[test]
fn test_rate_calculation_bonus_applied() {
    let base_rate = 1000;
    let rep_score = 80;
    let threshold = 80;
    let bonus = 200;
    let min_rate = 100;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 800);
}

#[test]
fn test_rate_calculation_no_bonus() {
    let base_rate = 1000;
    let rep_score = 79;
    let threshold = 80;
    let bonus = 200;
    let min_rate = 100;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 1000);
}

#[test]
fn test_rate_calculation_floor_enforced() {
    let base_rate = 300;
    let rep_score = 90;
    let threshold = 80;
    let bonus = 250;
    let min_rate = 100;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 100);
}

#[test]
fn test_exact_threshold_match() {
    let base_rate = 500;
    let rep_score = 50;
    let threshold = 50;
    let bonus = 100;
    let min_rate = 50;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 400);
}

#[test]
fn test_zero_reputation() {
    let base_rate = 500;
    let rep_score = 0;
    let threshold = 50;
    let bonus = 100;
    let min_rate = 50;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 500);
}

#[test]
fn test_maximum_bonus_application() {
    let base_rate = 600;
    let rep_score = 99;
    let threshold = 50;
    let bonus = 500;
    let min_rate = 50;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 100);
}

#[test]
fn test_zero_base_rate() {
    let base_rate = 0;
    let rep_score = 90;
    let threshold = 50;
    let bonus = 200;
    let min_rate = 50;

    let res = calculate_effective_rate(base_rate, rep_score, threshold, bonus, min_rate).unwrap();
    assert_eq!(res, 50);
}

#[test]
fn test_governance_setters_and_access_control() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let contract_id = env.register(ReputationBonusContract, ());
    let client = ReputationBonusContractClient::new(&env, &contract_id);

    client.init(&admin);

    let initial_config = Config {
        high_rep_threshold: 80,
        bonus_bps: 200,
        min_discount_rate_bps: 100,
    };

    client.set_config(&initial_config);

    let update_res = client.try_update_config(&non_admin, &90, &300, &150);
    assert!(update_res.is_err()); // non_admin update fails due to check

    let update_res_admin = client.try_update_config(&admin, &90, &300, &150);
    assert!(update_res_admin.is_ok());

    let events = env.events().all().filter_by_contract(&client.address);
    let expected = [
        ParameterUpdated {
            param_name: Symbol::new(&env, "high_rep_threshold"),
            old_value: 80,
            new_value: 90,
            updated_by: admin.clone(),
        }
        .to_xdr(&env, &client.address),
        ParameterUpdated {
            param_name: Symbol::new(&env, "bonus_bps"),
            old_value: 200,
            new_value: 300,
            updated_by: admin.clone(),
        }
        .to_xdr(&env, &client.address),
        ParameterUpdated {
            param_name: Symbol::new(&env, "min_discount_rate_bps"),
            old_value: 100,
            new_value: 150,
            updated_by: admin.clone(),
        }
        .to_xdr(&env, &client.address),
    ];

    assert_eq!(events.events().len(), expected.len());
    for (idx, expected_event) in expected.iter().enumerate() {
        assert_eq!(events.events().get(idx), Some(expected_event));
    }

    let config = client.get_config();
    assert_eq!(config.high_rep_threshold, 90);
    assert_eq!(config.bonus_bps, 300);
    assert_eq!(config.min_discount_rate_bps, 150);

    let invalid_bonus_res = client.try_update_config(&admin, &90, &501, &150);
    assert!(invalid_bonus_res.is_err()); // Capped at 500

    let invalid_min_rate_res = client.try_update_config(&admin, &90, &300, &0);
    assert!(invalid_min_rate_res.is_err()); // Min rate > 0
}

#[test]
fn test_submit_invoice_flow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    let contract_id = env.register(ReputationBonusContract, ());
    let client = ReputationBonusContractClient::new(&env, &contract_id);

    client.init(&admin);

    let config = Config {
        high_rep_threshold: 80,
        bonus_bps: 150,
        min_discount_rate_bps: 50,
    };
    client.set_config(&config);

    let freelancer = Address::generate(&env);
    let payer = Address::generate(&env);

    // 0 rep initially
    let inv1 = client.submit_invoice(&freelancer, &payer, &10_000, &1700000000, &400);
    assert_eq!(inv1.base_discount_rate_bps, 400);
    assert_eq!(inv1.effective_discount_rate_bps, 400);

    // Pay to get 100 rep
    client.mark_paid(&inv1.id);
    
    let inv2 = client.submit_invoice(&freelancer, &payer, &10_000, &1700000000, &400);
    assert_eq!(inv2.base_discount_rate_bps, 400);
    assert_eq!(inv2.effective_discount_rate_bps, 250); // Bonus applied (400 - 150)
}
