//! Tests for Issue #59 (GovernanceProposal struct),
//!           Issue #61 (cast_vote with anti-double-vote protection and VoteCast event),
//!       and Issue #64 (delegate_votes / undelegate_votes with transitive delegation).

#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{
        storage::Temporary,
        Address as _, Events, Ledger,
    },
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env,
};

// ── Test helpers ──────────────────────────────────────────────────────────────

struct GovTestEnv {
    env: Env,
    contract: GovContractClient<'static>,
    gov_token: TokenClient<'static>,
    gov_token_admin: StellarAssetClient<'static>,
    voter_a: Address,
    voter_b: Address,
    proposer: Address,
}

fn setup() -> GovTestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_addr = token_id.address();

    let gov_token = TokenClient::new(&env, &token_addr);
    let gov_token_admin = StellarAssetClient::new(&env, &token_addr);

    let voter_a = Address::generate(&env);
    let voter_b = Address::generate(&env);
    let proposer = Address::generate(&env);

    gov_token_admin.mint(&voter_a, &1_000);
    gov_token_admin.mint(&voter_b, &2_000);
    gov_token_admin.mint(&proposer, &500);

    let iln_contract = Address::generate(&env);

    let contract_id = env.register(GovContract, ());
    let contract = GovContractClient::new(&env, &contract_id);

    contract.initialize(&iln_contract, &token_addr);

    let mut ledger = env.ledger().get();
    ledger.timestamp = 1_700_000_000;
    env.ledger().set(ledger);

    GovTestEnv { env, contract, gov_token, gov_token_admin, voter_a, voter_b, proposer }
}

fn dummy_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[1u8; 32])
}

fn create_fee_proposal(t: &GovTestEnv) -> u64 {
    t.contract.create_proposal(
        &t.proposer,
        &ProposalAction::UpdateFeeRate(200),
        &dummy_hash(&t.env),
        &200_i128,
    )
}

// ── Issue #59 ─────────────────────────────────────────────────────────────────

#[test]
fn test_create_proposal_stores_correct_fields() {
    let t = setup();
    let hash = dummy_hash(&t.env);
    let now = t.env.ledger().timestamp();

    let id = t.contract.create_proposal(
        &t.proposer,
        &ProposalAction::UpdateFeeRate(300),
        &hash,
        &300_i128,
    );

    let p = t.contract.get_proposal(&id);

    assert_eq!(p.id, id);
    assert_eq!(p.proposer, t.proposer);
    assert_eq!(p.description_hash, hash);
    assert_eq!(p.action_type, ProposalAction::UpdateFeeRate(300));
    assert_eq!(p.proposed_value, 300);
    assert_eq!(p.status, ProposalStatus::Active);
    assert_eq!(p.votes_for, 0);
    assert_eq!(p.votes_against, 0);
    assert_eq!(p.created_at, now);
    assert_eq!(p.voting_end, now + 259_200);
}

#[test]
fn test_proposal_ids_increment() {
    let t = setup();
    let id1 = create_fee_proposal(&t);
    let id2 = create_fee_proposal(&t);
    assert_eq!(id2, id1 + 1);
}

#[test]
#[should_panic]
fn test_get_proposal_not_found_returns_error() {
    let t = setup();
    t.contract.get_proposal(&9999);
}

#[test]
fn test_proposal_action_add_token_stored_correctly() {
    let t = setup();
    let token_addr = Address::generate(&t.env);

    let id = t.contract.create_proposal(
        &t.proposer,
        &ProposalAction::AddToken(token_addr.clone()),
        &dummy_hash(&t.env),
        &0_i128,
    );

    let p = t.contract.get_proposal(&id);
    assert_eq!(p.action_type, ProposalAction::AddToken(token_addr));
    assert_eq!(p.proposed_value, 0);
}

#[test]
fn test_proposal_action_remove_token_stored_correctly() {
    let t = setup();
    let token_addr = Address::generate(&t.env);

    let id = t.contract.create_proposal(
        &t.proposer,
        &ProposalAction::RemoveToken(token_addr.clone()),
        &dummy_hash(&t.env),
        &0_i128,
    );

    let p = t.contract.get_proposal(&id);
    assert_eq!(p.action_type, ProposalAction::RemoveToken(token_addr));
}

#[test]
#[should_panic]
fn test_double_initialize_rejected() {
    let t = setup();
    let iln = Address::generate(&t.env);
    let token = Address::generate(&t.env);
    t.contract.initialize(&iln, &token);
}

// ── Issue #61 ─────────────────────────────────────────────────────────────────

#[test]
fn test_cast_vote_for_updates_votes_for() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, 1_000);
    assert_eq!(p.votes_against, 0);
}

#[test]
fn test_cast_vote_against_updates_votes_against() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &false);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_against, 1_000);
    assert_eq!(p.votes_for, 0);
}

#[test]
fn test_proposal_creation_snapshots_proposer_balance() {
    let t = setup();
    let id = create_fee_proposal(&t);

    let snapshot_key = StorageKey::VoteWeightSnapshot(id, t.proposer.clone());
    let snapshot: i128 = t
        .env
        .as_contract(&t.contract.address, || {
            t.env.storage().persistent().get(&snapshot_key).unwrap()
        });

    assert_eq!(snapshot, t.gov_token.balance(&t.proposer));
}

#[test]
fn test_cast_vote_uses_snapshotted_balance_after_balance_increase() {
    let t = setup();
    let id = t.contract.create_proposal(
        &t.proposer,
        &ProposalAction::UpdateFeeRate(200),
        &dummy_hash(&t.env),
        &200_i128,
    );

    let proposer_balance_before = t.gov_token.balance(&t.proposer);
    t.gov_token_admin.mint(&t.proposer, &2_000);

    t.contract.cast_vote(&t.proposer, &id, &true);

    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, proposer_balance_before);
    assert_eq!(p.votes_against, 0);
}

#[test]
fn test_cast_vote_weight_equals_token_balance() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_b, &id, &true);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, 2_000);
}

#[test]
fn test_multiple_voters_accumulate_correctly() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    t.contract.cast_vote(&t.voter_b, &id, &true);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, 3_000);
}

#[test]
fn test_has_voted_returns_true_after_vote() {
    let t = setup();
    let id = create_fee_proposal(&t);
    assert!(!t.contract.has_voted(&t.voter_a, &id));
    t.contract.cast_vote(&t.voter_a, &id, &true);
    assert!(t.contract.has_voted(&t.voter_a, &id));
}

#[test]
fn test_vote_receipt_uses_temporary_storage_with_ttl() {
    let t = setup();
    let id = create_fee_proposal(&t);
    let key = StorageKey::HasVoted(id, t.voter_a.clone());

    t.contract.cast_vote(&t.voter_a, &id, &true);

    let (temporary_has_receipt, persistent_has_receipt, ttl) =
        t.env.as_contract(&t.contract.address, || {
            (
                t.env.storage().temporary().has(&key),
                t.env.storage().persistent().has(&key),
                t.env.storage().temporary().get_ttl(&key),
            )
        });

    assert!(temporary_has_receipt);
    assert!(!persistent_has_receipt);
    assert!(ttl >= VOTE_RECEIPT_TTL_THRESHOLD_LEDGERS);
}

#[test]
fn test_vote_receipt_available_within_ttl() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);

    let mut ledger = t.env.ledger().get();
    ledger.sequence_number += VOTE_RECEIPT_TTL_THRESHOLD_LEDGERS - 1;
    ledger.timestamp += 1;
    t.env.ledger().set(ledger);

    assert!(t.contract.has_voted(&t.voter_a, &id));
}

#[test]
#[should_panic]
fn test_double_vote_rejected_with_already_voted_error() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    t.contract.cast_vote(&t.voter_a, &id, &false);
}

#[test]
fn test_double_vote_does_not_change_vote_counts() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_against, 0);
    assert_eq!(p.votes_for, 1_000);
}

#[test]
#[should_panic]
fn test_vote_on_nonexistent_proposal_rejected() {
    let t = setup();
    t.contract.cast_vote(&t.voter_a, &9999, &true);
}

#[test]
#[should_panic]
fn test_vote_after_voting_window_rejected() {
    let t = setup();
    let id = create_fee_proposal(&t);
    let mut ledger = t.env.ledger().get();
    ledger.timestamp += 259_201;
    t.env.ledger().set(ledger);
    t.contract.cast_vote(&t.voter_a, &id, &true);
}

#[test]
#[should_panic]
fn test_voter_with_zero_balance_rejected() {
    let t = setup();
    let id = create_fee_proposal(&t);
    let zero_voter = Address::generate(&t.env);
    t.contract.cast_vote(&zero_voter, &id, &true);
}

#[test]
fn test_cast_vote_emits_vote_cast_event() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    let events = t.env.events().all().filter_by_contract(&t.contract.address);
    assert!(!events.events().is_empty(), "VoteCast event should be emitted");
}

#[test]
#[should_panic]
fn test_execute_before_voting_ends_fails() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    t.contract.execute_proposal(&id, &10_000);
}

#[test]
#[should_panic]
fn test_execute_quorum_not_reached_rejected() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    let mut ledger = t.env.ledger().get();
    ledger.timestamp += 259_201;
    t.env.ledger().set(ledger);
    t.contract.execute_proposal(&id, &100_000);
}

#[test]
#[should_panic]
fn test_proposal_rejected_when_against_wins() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    t.contract.cast_vote(&t.voter_b, &id, &false);
    let mut ledger = t.env.ledger().get();
    ledger.timestamp += 259_201;
    t.env.ledger().set(ledger);
    t.contract.execute_proposal(&id, &3_000);
}

#[test]
#[should_panic]
fn test_already_resolved_proposal_cannot_be_executed_again() {
    let t = setup();
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_a, &id, &true);
    let mut ledger = t.env.ledger().get();
    ledger.timestamp += 259_201;
    t.env.ledger().set(ledger);
    t.contract.execute_proposal(&id, &100_000);
    t.contract.execute_proposal(&id, &100_000);
}

// ── Issue #64: delegate_votes / undelegate_votes ──────────────────────────────

#[test]
fn test_delegation_increases_delegate_vote_weight() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_b, &id, &true);
    let p = t.contract.get_proposal(&id);
    // voter_b own 2_000 + voter_a delegated 1_000 = 3_000
    assert_eq!(p.votes_for, 3_000);
}

#[test]
fn test_undelegation_removes_delegated_weight() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    t.contract.undelegate_votes(&t.voter_a);
    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&t.voter_b, &id, &true);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, 2_000); // only voter_b's own tokens
}

#[test]
fn test_get_delegate_returns_correct_address() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    let delegate = t.contract.get_delegate(&t.voter_a);
    assert_eq!(delegate, Some(t.voter_b.clone()));
}

#[test]
fn test_get_delegate_returns_none_after_undelegation() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    t.contract.undelegate_votes(&t.voter_a);
    let delegate = t.contract.get_delegate(&t.voter_a);
    assert_eq!(delegate, None);
}

#[test]
fn test_transitive_delegation_a_to_b_to_c() {
    let t = setup();
    let voter_c = Address::generate(&t.env);
    t.gov_token_admin.mint(&voter_c, &3_000);

    // B → C first, then A → B
    t.contract.delegate_votes(&t.voter_b, &voter_c);
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);

    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&voter_c, &id, &true);

    let p = t.contract.get_proposal(&id);
    // C own 3_000 + B delegated 2_000 + A delegated 1_000 = 6_000
    assert_eq!(p.votes_for, 6_000);
}

#[test]
#[should_panic]
fn test_cycle_prevention_direct_a_b_b_a() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    t.contract.delegate_votes(&t.voter_b, &t.voter_a); // must panic
}

#[test]
#[should_panic]
fn test_delegate_to_self_rejected() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_a);
}

#[test]
#[should_panic]
fn test_cycle_prevention_indirect_a_b_c_a() {
    let t = setup();
    let voter_c = Address::generate(&t.env);
    t.gov_token_admin.mint(&voter_c, &500);

    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    t.contract.delegate_votes(&t.voter_b, &voter_c);
    t.contract.delegate_votes(&voter_c, &t.voter_a); // must panic
}

#[test]
fn test_redelegation_moves_weight_to_new_delegate() {
    let t = setup();
    let voter_c = Address::generate(&t.env);
    t.gov_token_admin.mint(&voter_c, &500);

    t.contract.delegate_votes(&t.voter_a, &t.voter_b); // A → B
    t.contract.delegate_votes(&t.voter_a, &voter_c);   // A → C (re-delegate)

    let id = create_fee_proposal(&t);

    t.contract.cast_vote(&t.voter_b, &id, &false);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_against, 2_000); // B own only

    t.contract.cast_vote(&voter_c, &id, &true);
    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, 1_500); // C own 500 + A delegated 1_000
}

#[test]
fn test_delegate_votes_emits_votes_delegated_event() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    let events = t.env.events().all().filter_by_contract(&t.contract.address);
    assert!(!events.events().is_empty(), "VotesDelegated event should be emitted");
}

#[test]
fn test_undelegate_votes_emits_votes_undelegated_event() {
    let t = setup();
    t.contract.delegate_votes(&t.voter_a, &t.voter_b);
    t.contract.undelegate_votes(&t.voter_a);
    let events = t.env.events().all().filter_by_contract(&t.contract.address);
    assert!(events.events().len() >= 2, "VotesUndelegated event should be emitted");
}

#[test]
fn test_zero_balance_voter_with_delegation_can_vote() {
    let t = setup();
    let receiver = Address::generate(&t.env);
    // receiver has 0 own tokens

    t.contract.delegate_votes(&t.voter_a, &receiver);

    let id = create_fee_proposal(&t);
    t.contract.cast_vote(&receiver, &id, &true);

    let p = t.contract.get_proposal(&id);
    assert_eq!(p.votes_for, 1_000); // only delegated weight from voter_a
}