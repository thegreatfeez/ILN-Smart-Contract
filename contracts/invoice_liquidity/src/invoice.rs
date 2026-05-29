use crate::storage::DataKey as StorageKey;
use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol};

// ----------------------------------------------------------------
// Status enum — tracks lifecycle of invoice
// ----------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum InvoiceStatus {
    Pending,         // submitted, waiting for a liquidity provider to fund it
    Funded,          // LP has funded it, freelancer has been paid out
    PartiallyFunded, // partially funded by one or more LPs
    Paid,            // payer has settled in full, LP has been released
    Defaulted,       // past due_date and still unpaid
    Appealed,        // payer has contested the default ruling (issue #36)
    Disputed,        // payer has disputed the invoice before settlement
    Expired,         // past due_date with no funding
    Cancelled,       // freelancer cancelled the invoice before funding
}

// ----------------------------------------------------------------
// Invoice struct (UPDATED - token stays per invoice)
// ----------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Invoice {
    pub id: u64,
    pub freelancer: Address, // who submitted the invoice (receives liquidity)
    pub payer: Address,      // the client who owes the money
    pub token: Address,      // token used for this invoice lifecycle
    pub amount: i128,        // full invoice value in stroops (1 USDC = 10_000_000)
    pub due_date: u32,       // Unix timestamp — when the payer must settle by
    pub discount_rate: u32,  // basis points, e.g. 300 = 3.00%
    pub status: InvoiceStatus,
    pub funder: Option<Address>, // set when an LP funds the invoice (legacy for full funding)
    pub funded_at: Option<u32>,  // ledger timestamp when funding occurred
    pub amount_funded: i128,     // cumulative amount funded so far
    pub amount_paid: i128,       // cumulative amount paid by the payer
    pub submitter_reputation: u32, // snapshot of freelancer's reputation at submission time
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct InvoiceParams {
    pub freelancer: Address,
    pub payer: Address,
    pub amount: i128,
    pub due_date: u64,
    pub discount_rate: u32,
    pub token: Address,
}

#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct PayerStats {
    pub total_invoices: u64,
    pub paid_on_time: u64,
    pub defaults: u64,
    pub total_volume: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ReputationScore {
    pub score: u32,
    pub last_activity_ledger: u32,
}

/// Detailed reputation profile for an address (Issue #26).
///
/// Foundational data model for the reputation system. The existing
/// [`ReputationScore`] holds the lightweight decaying score used by the
/// payer/LP scoring path; this profile records the richer counters that future
/// reputation logic builds on. Unknown addresses resolve to a zeroed profile
/// (see [`get_reputation`]) rather than panicking.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReputationProfile {
    pub address: Address,
    pub invoices_submitted: u32,
    pub invoices_paid: u32,
    pub invoices_defaulted: u32,
    pub score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct ContractStats {
    pub total_invoices: u64,
    pub total_funded: u64,
    pub total_paid: u64,
    pub total_volume_usdc: i128,
    pub total_volume_eurc: i128,
    pub total_volume_xlm: i128,
    pub token_volumes: soroban_sdk::Vec<(Address, i128)>,
    pub total_volume_usd_normalized: i128,
}

// ----------------------------------------------------------------
// Issue #36: Appeal record stored per invoice
// ----------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct AppealRecord {
    /// SHA-256 hash of off-chain evidence submitted by the payer.
    pub evidence_hash: BytesN<32>,
    /// Ledger timestamp when the appeal was filed.
    pub appealed_at: u32,
    /// Payer reputation score just before the default was applied,
    /// used to restore the score if the appeal is upheld.
    pub pre_default_score: u32,
}

// ----------------------------------------------------------------
// Dispute record stored per invoice
// ----------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct DisputeRecord {
    /// SHA-256 hash of off-chain dispute evidence.
    pub reason_hash: BytesN<32>,
    /// Ledger sequence when the dispute was filed.
    pub disputed_at: u32,
}

// ----------------------------------------------------------------
// Issue #34: Single entry in the LP priority queue
// ----------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct LpFundRequest {
    pub lp: Address,
    /// LP reputation score snapshotted at request time (used for ordering).
    pub score: u32,
}

// ----------------------------------------------------------------
// Storage helpers — core invoice CRUD
// ----------------------------------------------------------------

pub fn get_submitter_invoices(env: &Env, submitter: &Address) -> soroban_sdk::Vec<u64> {
    env.storage()
        .persistent()
        .get(&StorageKey::SubmitterInvoices(submitter.clone()))
        .unwrap_or(soroban_sdk::Vec::new(env))
}

pub fn add_invoice_to_submitter(env: &Env, submitter: &Address, invoice_id: u64) {
    let mut invoices = get_submitter_invoices(env, submitter);
    invoices.push_back(invoice_id);
    let key = StorageKey::SubmitterInvoices(submitter.clone());
    env.storage().persistent().set(&key, &invoices);
    env.storage()
        .persistent()
        .extend_ttl(&key, 1_000_000, 2_000_000);
}

pub fn remove_invoice_from_submitter(env: &Env, submitter: &Address, invoice_id: u64) {
    let invoices = get_submitter_invoices(env, submitter);
    let mut new_invoices = soroban_sdk::Vec::new(env);
    for id in invoices.iter() {
        if id != invoice_id {
            new_invoices.push_back(id);
        }
    }
    let key = StorageKey::SubmitterInvoices(submitter.clone());
    env.storage().persistent().set(&key, &new_invoices);
    env.storage()
        .persistent()
        .extend_ttl(&key, 1_000_000, 2_000_000);
}

pub fn get_lp_invoices(env: &Env, lp: &Address) -> soroban_sdk::Vec<u64> {
    env.storage()
        .persistent()
        .get(&StorageKey::LpInvoices(lp.clone()))
        .unwrap_or(soroban_sdk::Vec::new(env))
}

pub fn add_invoice_to_lp(env: &Env, lp: &Address, invoice_id: u64) {
    let mut invoices = get_lp_invoices(env, lp);
    // Check if already present to avoid duplicates in case of partial funding
    let mut exists = false;
    for id in invoices.iter() {
        if id == invoice_id {
            exists = true;
            break;
        }
    }
    if !exists {
        invoices.push_back(invoice_id);
        let key = StorageKey::LpInvoices(lp.clone());
        env.storage().persistent().set(&key, &invoices);
        env.storage()
            .persistent()
            .extend_ttl(&key, 1_000_000, 2_000_000);
    }
}

pub fn save_invoice(env: &Env, invoice: &Invoice) {
    let key = StorageKey::Invoice(invoice.id);
    env.storage().persistent().set(&key, invoice);
    env.storage()
        .persistent()
        .extend_ttl(&key, 1_000_000, 2_000_000);
}

pub fn load_invoice(env: &Env, id: u64) -> Invoice {
    env.storage()
        .persistent()
        .get(&StorageKey::Invoice(id))
        .expect("invoice not found")
}

pub fn invoice_exists(env: &Env, id: u64) -> bool {
    env.storage().persistent().has(&StorageKey::Invoice(id))
}

/// Load an invoice in a single storage read, returning `None` if it does not
/// exist (Issue #71). Prefer this over the `invoice_exists` + `load_invoice`
/// pair in hot paths, which reads the same key twice.
pub fn try_load_invoice(env: &Env, id: u64) -> Option<Invoice> {
    env.storage().persistent().get(&StorageKey::Invoice(id))
}

pub fn next_invoice_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&StorageKey::InvoiceCount)
        .unwrap_or(0);

    let next = current + 1;

    env.storage()
        .persistent()
        .set(&StorageKey::InvoiceCount, &next);
    env.storage()
        .persistent()
        .extend_ttl(&StorageKey::InvoiceCount, 1_000_000, 2_000_000);

    next
}

// ----------------------------------------------------------------
// Reputation Score
// ----------------------------------------------------------------

/// Get a payer's reputation score (0-100, default 50)
pub fn get_payer_score(env: &Env, payer: &Address) -> u32 {
    match env
        .storage()
        .persistent()
        .get::<StorageKey, ReputationScore>(&StorageKey::PayerScore(payer.clone()))
    {
        Some(mut rep) => {
            // Apply decay if enough ledgers have passed and config exists
            if let Some(decay_config) = crate::storage::get_config(env) {
                let current_ledger = env.ledger().sequence();
                let ledgers_since_activity =
                    current_ledger.saturating_sub(rep.last_activity_ledger);

                if u64::from(ledgers_since_activity) >= decay_config.decay_period_ledgers
                    && decay_config.decay_period_ledgers > 0
                    && decay_config.decay_rate_bps > 0
                {
                    // Calculate number of decay periods that have passed
                    let periods_passed =
                        u64::from(ledgers_since_activity) / decay_config.decay_period_ledgers;

                    // Apply decay: score = score * (1 - decay_rate/10000)^periods
                    let mut decayed_score = rep.score as u64;
                    for _ in 0..periods_passed {
                        // Decay: subtract decay_rate_bps basis points (min 1 point)
                        let mut decay_amount =
                            (decayed_score * decay_config.decay_rate_bps as u64) / 10_000;
                        if decay_amount == 0 && decayed_score > 0 {
                            decay_amount = 1;
                        }
                        decayed_score = decayed_score.saturating_sub(decay_amount);
                    }

                    rep.score = (decayed_score.min(100)) as u32;
                }
            }

            rep.score
        }
        None => 50, // Default neutral score for new users
    }
}

/// Update a payer's reputation score (capped at 100)
pub fn set_payer_score(env: &Env, payer: &Address, score: u32) {
    let score = score.min(100);
    let rep = ReputationScore {
        score,
        last_activity_ledger: env.ledger().sequence(),
    };
    env.storage()
        .persistent()
        .set(&StorageKey::PayerScore(payer.clone()), &rep);
}

// ----------------------------------------------------------------
// Issue #26: Reputation profile (detailed model)
// ----------------------------------------------------------------

/// Read an address's detailed reputation profile. Unknown addresses return a
/// zeroed profile (no panic) so callers can branch on the counters directly.
pub fn get_reputation(env: &Env, address: &Address) -> ReputationProfile {
    env.storage()
        .persistent()
        .get(&StorageKey::Reputation(address.clone()))
        .unwrap_or(ReputationProfile {
            address: address.clone(),
            invoices_submitted: 0,
            invoices_paid: 0,
            invoices_defaulted: 0,
            score: 0,
        })
}

/// Persist an address's reputation profile.
pub fn set_reputation(env: &Env, profile: &ReputationProfile) {
    let key = StorageKey::Reputation(profile.address.clone());
    env.storage().persistent().set(&key, profile);
    env.storage()
        .persistent()
        .extend_ttl(&key, 1_000_000, 2_000_000);
}

// ----------------------------------------------------------------
// Issue #28: Minimum payer reputation threshold
// ----------------------------------------------------------------

/// Minimum payer reputation required to fund an invoice. Defaults to 0
/// (allowing all payers) when unset.
pub fn get_min_payer_reputation(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::MinPayerReputation)
        .unwrap_or(0)
}

/// Set the minimum payer reputation threshold.
pub fn set_min_payer_reputation(env: &Env, value: u32) {
    env.storage()
        .instance()
        .set(&StorageKey::MinPayerReputation, &value);
}

// ----------------------------------------------------------------
// Funder list helpers
// ----------------------------------------------------------------

/// Get the list of funders and their contributions for an invoice
pub fn get_invoice_funders(env: &Env, id: u64) -> soroban_sdk::Vec<(Address, i128)> {
    env.storage()
        .persistent()
        .get(&StorageKey::InvoiceFunders(id))
        .unwrap_or(soroban_sdk::Vec::new(env))
}

/// Save the list of funders for an invoice
pub fn save_invoice_funders(env: &Env, id: u64, funders: &soroban_sdk::Vec<(Address, i128)>) {
    env.storage()
        .persistent()
        .set(&StorageKey::InvoiceFunders(id), funders);
}

// ----------------------------------------------------------------
// Issue #36: Appeal helpers
// ----------------------------------------------------------------

pub fn get_appeal(env: &Env, invoice_id: u64) -> Option<AppealRecord> {
    env.storage()
        .persistent()
        .get(&StorageKey::Appeal(invoice_id))
}

pub fn save_appeal(env: &Env, invoice_id: u64, record: &AppealRecord) {
    env.storage()
        .persistent()
        .set(&StorageKey::Appeal(invoice_id), record);
}

/// Store the payer's score BEFORE the default penalty is applied.
/// Called inside claim_default() so appeal_default() can restore it later.
pub fn save_pre_default_payer_score(env: &Env, invoice_id: u64, score: u32) {
    env.storage()
        .persistent()
        .set(&StorageKey::PreDefaultPayerScore(invoice_id), &score);
}

pub fn get_pre_default_payer_score(env: &Env, invoice_id: u64) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&StorageKey::PreDefaultPayerScore(invoice_id))
}

// Dispute helpers

pub fn get_dispute(env: &Env, invoice_id: u64) -> Option<DisputeRecord> {
    env.storage()
        .persistent()
        .get(&StorageKey::Dispute(invoice_id))
}

pub fn save_dispute(env: &Env, invoice_id: u64, record: &DisputeRecord) {
    env.storage()
        .persistent()
        .set(&StorageKey::Dispute(invoice_id), record);
}

// ----------------------------------------------------------------
// Issue #34: LP score + queue helpers
// ----------------------------------------------------------------

/// LP reputation score starts at 50 (same neutral baseline as payers)
pub fn get_lp_score(env: &Env, lp: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&StorageKey::LpScore(lp.clone()))
        .unwrap_or(50)
}

/// Update an LP's reputation score (capped at 100)
pub fn set_lp_score(env: &Env, lp: &Address, score: u32) {
    let score = score.min(100);
    env.storage()
        .persistent()
        .set(&StorageKey::LpScore(lp.clone()), &score);
}

/// Return all queued LP requests for an invoice
pub fn get_fund_queue(env: &Env, invoice_id: u64) -> soroban_sdk::Vec<LpFundRequest> {
    env.storage()
        .persistent()
        .get(&StorageKey::FundQueue(invoice_id))
        .unwrap_or(soroban_sdk::Vec::new(env))
}

/// Persist the queue
pub fn save_fund_queue(env: &Env, invoice_id: u64, queue: &soroban_sdk::Vec<LpFundRequest>) {
    env.storage()
        .persistent()
        .set(&StorageKey::FundQueue(invoice_id), queue);
}

/// Return the resolved (approved) funder for an invoice, if any
pub fn get_queue_resolution(env: &Env, invoice_id: u64) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&StorageKey::QueueResolution(invoice_id))
}

/// Store the approved funder chosen by the priority queue
pub fn save_queue_resolution(env: &Env, invoice_id: u64, approved_lp: &Address) {
    env.storage()
        .persistent()
        .set(&StorageKey::QueueResolution(invoice_id), approved_lp);
}
// Contract stats helpers
// ----------------------------------------------------------------

pub fn get_contract_stats(env: &Env) -> ContractStats {
    let token_list: soroban_sdk::Vec<Address> = env
        .storage()
        .persistent()
        .get(&StorageKey::TokenList)
        .unwrap_or(soroban_sdk::Vec::new(env));

    let mut token_volumes = soroban_sdk::Vec::new(env);
    let mut total_volume_usd_normalized: i128 = 0;

    for token in token_list.iter() {
        let volume: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::TokenVolume(token.clone()))
            .unwrap_or(0);
        token_volumes.push_back((token.clone(), volume));
        if let Some(price_bps) = get_price_from_oracle(env, &token) {
            total_volume_usd_normalized = total_volume_usd_normalized
                .checked_add(volume.checked_mul(price_bps).unwrap_or(0) / 10_000)
                .unwrap_or(total_volume_usd_normalized);
        }
    }

    ContractStats {
        total_invoices: env
            .storage()
            .persistent()
            .get(&StorageKey::TotalInvoices)
            .unwrap_or(0),
        total_funded: env
            .storage()
            .persistent()
            .get(&StorageKey::TotalFunded)
            .unwrap_or(0),
        total_paid: env
            .storage()
            .persistent()
            .get(&StorageKey::TotalPaid)
            .unwrap_or(0),
        total_volume_usdc: env
            .storage()
            .persistent()
            .get(&StorageKey::TotalVolumeUsdc)
            .unwrap_or(0),
        total_volume_eurc: env
            .storage()
            .persistent()
            .get(&StorageKey::TotalVolumeEurc)
            .unwrap_or(0),
        total_volume_xlm: env
            .storage()
            .persistent()
            .get(&StorageKey::TotalVolumeXlm)
            .unwrap_or(0),
        token_volumes,
        total_volume_usd_normalized,
    }
}

fn get_price_from_oracle(env: &Env, token: &Address) -> Option<i128> {
    let config = crate::storage::get_config(env)?;
    let oracle = config.price_oracle?;
    let args = soroban_sdk::vec![env, token.clone().into_val(env)];
    Some(env.invoke_contract::<i128>(&oracle, &Symbol::new(env, "get_price"), args))
}

pub fn add_volume(env: &Env, token: &Address, amount: i128) {
    // Track per-token volume in a mutable map.
    let current_per_token: i128 = env
        .storage()
        .persistent()
        .get(&StorageKey::TokenVolume(token.clone()))
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&StorageKey::TokenVolume(token.clone()), &(current_per_token + amount));

    // Preserve legacy aggregate token counters for compatibility.
    let token_list: soroban_sdk::Vec<Address> = env
        .storage()
        .persistent()
        .get(&StorageKey::TokenList)
        .unwrap_or(soroban_sdk::Vec::new(env));

    if token_list.len() > 0 {
        if let Some(usdc_addr) = token_list.get(0) {
            if token == &usdc_addr {
                let current: i128 = env
                    .storage()
                    .persistent()
                    .get(&StorageKey::TotalVolumeUsdc)
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&StorageKey::TotalVolumeUsdc, &(current + amount));
            }
        }
    }
    if token_list.len() > 1 {
        if let Some(eurc_addr) = token_list.get(1) {
            if token == &eurc_addr {
                let current: i128 = env
                    .storage()
                    .persistent()
                    .get(&StorageKey::TotalVolumeEurc)
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&StorageKey::TotalVolumeEurc, &(current + amount));
            }
        }
    }
    if token_list.len() > 2 {
        if let Some(xlm_addr) = token_list.get(2) {
            if token == &xlm_addr {
                let current: i128 = env
                    .storage()
                    .persistent()
                    .get(&StorageKey::TotalVolumeXlm)
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&StorageKey::TotalVolumeXlm, &(current + amount));
            }
        }
    }
}

pub fn increment_total_invoices(env: &Env) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&StorageKey::TotalInvoices)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&StorageKey::TotalInvoices, &(current + 1));
}

pub fn increment_total_funded(env: &Env) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&StorageKey::TotalFunded)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&StorageKey::TotalFunded, &(current + 1));
}

pub fn increment_total_paid(env: &Env) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&StorageKey::TotalPaid)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&StorageKey::TotalPaid, &(current + 1));
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&StorageKey::Paused)
        .unwrap_or(false)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&StorageKey::Paused, &paused);
}
