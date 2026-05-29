use soroban_sdk::{contracttype, Address, Env};

use crate::config::Config;
use crate::invoice::{AppealRecord, Invoice, LpFundRequest, ReputationScore};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    // Instance Storage
    Admin,
    Config,
    FeeRate,
    MaxDiscountRate,
    DistributionContract,
    Paused,
    /// Minimum payer reputation required to fund an invoice (Issue #28). Default 0.
    MinPayerReputation,

    // Persistent Storage
    Invoice(u64),
    InvoiceCount,
    Token,
    PayerScore(Address),
    InvoiceFunders(u64),
    ApprovedToken(Address),
    TokenList,
    /// Detailed reputation profile per address (Issue #26).
    Reputation(Address),
    Appeal(u64),
    PreDefaultPayerScore(u64),
    LpScore(Address),
    FundQueue(u64),
    QueueResolution(u64),

    // Stats (Persistent)
    TotalInvoices,
    TotalFunded,
    TotalPaid,
    TotalVolumeUsdc,
    TotalVolumeEurc,
    TotalVolumeXlm,
    TokenVolume(Address),
    Dispute(u64),
    SubmitterInvoices(Address),
    LpInvoices(Address),
}

// ----------------------------------------------------------------
// Config Helpers
// ----------------------------------------------------------------

pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Admin)
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_config(env: &Env) -> Option<Config> {
    env.storage().instance().get(&DataKey::Config)
}

pub fn set_config(env: &Env, config: &Config) {
    env.storage().instance().set(&DataKey::Config, config);
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

// ----------------------------------------------------------------
// Invoice Helpers
// ----------------------------------------------------------------

pub fn save_invoice(env: &Env, invoice: &Invoice) {
    let key = DataKey::Invoice(invoice.id);
    env.storage().persistent().set(&key, invoice);
    env.storage()
        .persistent()
        .extend_ttl(&key, 1_000_000, 2_000_000);
}

pub fn load_invoice(env: &Env, id: u64) -> Invoice {
    env.storage()
        .persistent()
        .get(&DataKey::Invoice(id))
        .expect("invoice not found")
}

pub fn invoice_exists(env: &Env, id: u64) -> bool {
    env.storage().persistent().has(&DataKey::Invoice(id))
}

pub fn read_next_invoice_id(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::NextInvoiceId)
        .unwrap_or(1)
}

pub fn write_next_invoice_id(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::NextInvoiceId, &id);
}

pub fn next_invoice_id(env: &Env) -> Result<u64, crate::errors::ContractError> {
    let current_id = read_next_invoice_id(env);
    let next_id = current_id
        .checked_add(1)
        .ok_or(crate::errors::ContractError::ArithmeticOverflow)?;

    write_next_invoice_id(env, next_id);

    Ok(current_id)
}

// ----------------------------------------------------------------
// Funder List Helpers
// ----------------------------------------------------------------

pub fn get_invoice_funders(env: &Env, id: u64) -> soroban_sdk::Vec<(Address, i128)> {
    env.storage()
        .persistent()
        .get(&DataKey::InvoiceFunders(id))
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

pub fn save_invoice_funders(env: &Env, id: u64, funders: &soroban_sdk::Vec<(Address, i128)>) {
    env.storage()
        .persistent()
        .set(&DataKey::InvoiceFunders(id), funders);
}

// ----------------------------------------------------------------
// Reputation Helpers
// ----------------------------------------------------------------

pub fn get_payer_score(env: &Env, payer: &Address) -> u32 {
    match env
        .storage()
        .persistent()
        .get::<DataKey, ReputationScore>(&DataKey::PayerScore(payer.clone()))
    {
        Some(mut rep) => {
            if let Some(decay_config) = get_config(env) {
                let current_ledger = env.ledger().sequence() as u64;
                let ledgers_since_activity =
                    current_ledger.saturating_sub(rep.last_activity_ledger.into());

                if ledgers_since_activity >= decay_config.decay_period_ledgers
                    && decay_config.decay_period_ledgers > 0
                    && decay_config.decay_rate_bps > 0
                {
                    let periods_passed = ledgers_since_activity / decay_config.decay_period_ledgers;
                    let mut decayed_score = rep.score as u64;
                    for _ in 0..periods_passed {
                        let decay_amount =
                            (decayed_score * decay_config.decay_rate_bps as u64) / 10_000;
                        decayed_score = decayed_score.saturating_sub(decay_amount);
                    }
                    rep.score = (decayed_score.min(100)) as u32;
                }
            }
            rep.score
        }
        None => 50,
    }
}

pub fn set_payer_score(env: &Env, payer: &Address, score: u32) {
    let score = score.min(100);
    // Note: To preserve `last_activity_ledger`, we should actually retrieve the old Rep or create a new one.
    // In `invoice.rs` the old function was `set_payer_score(env: &Env, payer: &Address, score: u32) { env.storage().persistent().set(..., &rep) }` which didn't compile correctly in the snippet I saw (`&rep` not defined). Let's fix that.
    let current_ledger = env.ledger().sequence() as u64;
    let rep = ReputationScore {
        score,
        last_activity_ledger: current_ledger as u32,
    };
    env.storage()
        .persistent()
        .set(&DataKey::PayerScore(payer.clone()), &rep);
}

pub fn get_lp_score(env: &Env, lp: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::LpScore(lp.clone()))
        .unwrap_or(50)
}

pub fn set_lp_score(env: &Env, lp: &Address, score: u32) {
    let score = score.min(100);
    env.storage()
        .persistent()
        .set(&DataKey::LpScore(lp.clone()), &score);
}

// ----------------------------------------------------------------
// LP Queue Helpers
// ----------------------------------------------------------------

pub fn get_fund_queue(env: &Env, invoice_id: u64) -> soroban_sdk::Vec<LpFundRequest> {
    env.storage()
        .persistent()
        .get(&DataKey::FundQueue(invoice_id))
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

pub fn save_fund_queue(env: &Env, invoice_id: u64, queue: &soroban_sdk::Vec<LpFundRequest>) {
    env.storage()
        .persistent()
        .set(&DataKey::FundQueue(invoice_id), queue);
}

pub fn get_queue_resolution(env: &Env, invoice_id: u64) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::QueueResolution(invoice_id))
}

pub fn save_queue_resolution(env: &Env, invoice_id: u64, approved_lp: &Address) {
    env.storage()
        .persistent()
        .set(&DataKey::QueueResolution(invoice_id), approved_lp);
}

// ----------------------------------------------------------------
// Appeal Helpers
// ----------------------------------------------------------------

pub fn get_appeal(env: &Env, invoice_id: u64) -> Option<AppealRecord> {
    env.storage().persistent().get(&DataKey::Appeal(invoice_id))
}

pub fn save_appeal(env: &Env, invoice_id: u64, record: &AppealRecord) {
    env.storage()
        .persistent()
        .set(&DataKey::Appeal(invoice_id), record);
}

pub fn save_pre_default_payer_score(env: &Env, invoice_id: u64, score: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::PreDefaultPayerScore(invoice_id), &score);
}

pub fn get_pre_default_payer_score(env: &Env, invoice_id: u64) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::PreDefaultPayerScore(invoice_id))
}

// ----------------------------------------------------------------
// Contract Stats Helpers
// ----------------------------------------------------------------



pub fn increment_total_invoices(env: &Env) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::TotalInvoices)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&DataKey::TotalInvoices, &(current + 1));
}

pub fn increment_total_funded(env: &Env) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::TotalFunded)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&DataKey::TotalFunded, &(current + 1));
}

pub fn increment_total_paid(env: &Env) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::TotalPaid)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&DataKey::TotalPaid, &(current + 1));
}

pub fn add_volume(
    env: &Env,
    token: &Address,
    amount: i128,
    usdc_addr: &Address,
    eurc_addr: &Address,
    xlm_addr: &Address,
) {
    if token == usdc_addr {
        let current: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalVolumeUsdc)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalVolumeUsdc, &(current + amount));
    } else if token == eurc_addr {
        let current: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalVolumeEurc)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalVolumeEurc, &(current + amount));
    } else if token == xlm_addr {
        let current: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalVolumeXlm)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalVolumeXlm, &(current + amount));
    }
}
