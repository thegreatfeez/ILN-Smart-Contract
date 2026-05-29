#![no_std]

pub mod access;
pub mod config;
pub mod errors;
pub mod events;
pub mod invoice;
pub mod rate_logic;
pub mod storage;
use access::*;
mod tests_lp_pagination;
mod tests_new_features;
mod tests_pagination;
mod tests_regression;
mod tests_xlm_support;

pub use crate::invoice::{
    AppealRecord, Invoice, InvoiceParams, InvoiceStatus, LpFundRequest, ReputationProfile,
    ReputationScore,
};
pub use crate::storage::DataKey;
pub use config::{Config, ConfigError};
pub use errors::ContractError;
use soroban_sdk::{
    contract, contractimpl, token::Client as TokenClient, vec, Address, BytesN, Env, IntoVal,
    Symbol, Vec,
};

use crate::storage::get_admin;
use events::{
    AdminChanged, AppealResolved, ContractPaused, ContractUnpaused, ContractUpgraded,
    DefaultAppealed, DisputeResolved, FundQueueResolved, FundRequested, InvoiceCancelled,
    InvoiceDefaulted, InvoiceDisputed, InvoiceFunded, InvoicePaid, InvoicePartiallyPaid,
    InvoiceSubmitted, InvoiceTransferred, InvoiceUpdated, ParameterUpdated, TokenAdded,
    TokenRemoved,
};
use invoice::{
    add_invoice_to_lp, add_invoice_to_submitter, add_volume, get_appeal, get_contract_stats,
    get_dispute, get_fund_queue, get_invoice_funders, get_lp_invoices, get_lp_score,
    get_min_payer_reputation, get_payer_score, get_pre_default_payer_score, get_queue_resolution,
    get_reputation, get_submitter_invoices, increment_total_funded, increment_total_invoices,
    increment_total_paid, invoice_exists, is_paused, load_invoice, next_invoice_id,
    remove_invoice_from_submitter, save_appeal, save_dispute, save_fund_queue, save_invoice,
    save_invoice_funders, save_pre_default_payer_score, save_queue_resolution, set_lp_score,
    set_min_payer_reputation, set_paused, set_payer_score, try_load_invoice, ContractStats,
    DisputeRecord, StorageKey,
};
// 30-day window in seconds for a payer to file an appeal after a default.
const APPEAL_WINDOW_SECONDS: u64 = 30 * 24 * 60 * 60;

// ----------------------------------------------------------------
// CONSTANTS
// ----------------------------------------------------------------

/// Minimum invoice duration: 24 hours (in seconds)
const MIN_INVOICE_DURATION: u64 = 24 * 60 * 60;

/// Maximum invoice duration: 365 days (in seconds)
const MAX_INVOICE_DURATION: u64 = 365 * 24 * 60 * 60;

// ----------------------------------------------------------------
// CONTRACT
// ----------------------------------------------------------------

#[contract]
pub struct InvoiceLiquidityContract;

#[contractimpl]
impl InvoiceLiquidityContract {
    // ------------------------------------------------------------
    // initialize (multi-token aware)
    // ------------------------------------------------------------
    /// Access: Anyone
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        xlm_token: Address,
    ) -> Result<(), ContractError> {
        if env
            .storage()
            .instance()
            .has(&crate::storage::DataKey::InvoiceCount)
        {
            return Err(ContractError::AlreadyInitialized);
        }

        env.storage()
            .instance()
            .set(&crate::storage::DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&crate::storage::DataKey::FeeRate, &0_u32);
        env.storage()
            .instance()
            .set(&crate::storage::DataKey::MaxDiscountRate, &5000_u32);

        // Initialize config with XLM SAC address
        let initial_config = crate::config::Config {
            high_rep_threshold: 70,
            bonus_bps: 100,
            min_discount_rate_bps: 100,
            decay_rate_bps: 50,
            decay_period_ledgers: 10000,
            dispute_timeout_ledgers: 10000,
            xlm_sac_address: xlm_token.clone(),
            price_oracle: None,
        };
        crate::storage::set_config(&env, &initial_config);

        // approve first token (USDC or default)
        env.storage().persistent().set(
            &crate::storage::DataKey::ApprovedToken(token.clone()),
            &true,
        );

        // approve native XLM SAC
        env.storage().persistent().set(
            &crate::storage::DataKey::ApprovedToken(xlm_token.clone()),
            &true,
        );

        let mut list: Vec<Address> = Vec::new(&env);
        list.push_back(token.clone());
        list.push_back(xlm_token.clone());

        env.storage()
            .persistent()
            .set(&crate::storage::DataKey::TokenList, &list);

        Ok(())
    }

    // ------------------------------------------------------------
    /// Access: Admin only
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        let old_admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        env.storage().instance().set(&StorageKey::Admin, &new_admin);
        env.events().publish_event(&AdminChanged {
            old_admin,
            new_admin,
            timestamp: env.ledger().timestamp(),
        });
        Ok(())
    }

    /// Access: Admin only
    pub fn update_fee_rate(env: Env, rate: u32) -> Result<(), ContractError> {
        require_admin(&env)?;

        let old_rate: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::FeeRate)
            .unwrap_or(0);
        env.storage().instance().set(&StorageKey::FeeRate, &rate);
        let updated_by = get_admin(&env).ok_or(ContractError::Unauthorized)?;
        env.events().publish_event(&ParameterUpdated {
            param_name: Symbol::new(&env, "protocol_fee_rate_bps"),
            old_value: old_rate as i128,
            new_value: rate as i128,
            updated_by,
        });
        Ok(())
    }

    /// Access: Admin only
    pub fn update_max_discount(env: Env, rate: u32) -> Result<(), ContractError> {
        require_admin(&env)?;

        let old_rate: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::MaxDiscountRate)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&StorageKey::MaxDiscountRate, &rate);
        let updated_by = get_admin(&env).ok_or(ContractError::Unauthorized)?;
        env.events().publish_event(&ParameterUpdated {
            param_name: Symbol::new(&env, "max_discount_rate_bps"),
            old_value: old_rate as i128,
            new_value: rate as i128,
            updated_by,
        });
        Ok(())
    }

    /// Access: Admin only
    pub fn set_distribution_contract(
        env: Env,
        distribution_contract: Address,
    ) -> Result<(), ContractError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&StorageKey::DistributionContract, &distribution_contract);
        Ok(())
    }

    /// Access: Admin only
    pub fn set_price_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        let admin = get_admin(&env).ok_or(ContractError::Unauthorized)?;
        crate::config::set_price_oracle(&env, &admin, oracle)?;
        Ok(())
    }

    /// Access: Anyone
    pub fn get_price_oracle(env: Env) -> Option<Address> {
        crate::storage::get_config(&env).and_then(|config| config.price_oracle)
    }

    /// Access: Admin only
    pub fn add_token(env: Env, token: Address) -> Result<(), ContractError> {
        require_admin(&env)?;

        env.storage().persistent().set(
            &crate::storage::DataKey::ApprovedToken(token.clone()),
            &true,
        );

        let mut list: Vec<Address> = env
            .storage()
            .persistent()
            .get(&crate::storage::DataKey::TokenList)
            .unwrap_or(Vec::new(&env));
        if !list.contains(&token) {
            list.push_back(token.clone());
            env.storage()
                .persistent()
                .set(&crate::storage::DataKey::TokenList, &list);
        }

        env.events().publish_event(&TokenAdded { token });
        Ok(())
    }

    /// Access: Admin only
    pub fn remove_token(env: Env, token: Address) -> Result<(), ContractError> {
        require_admin(&env)?;

        env.storage()
            .persistent()
            .set(&StorageKey::ApprovedToken(token.clone()), &false);

        // Keep the allowlist Vec in sync with the ApprovedToken flag.
        let list: Vec<Address> = env
            .storage()
            .persistent()
            .get(&crate::storage::DataKey::TokenList)
            .unwrap_or(Vec::new(&env));
        let mut pruned: Vec<Address> = Vec::new(&env);
        for t in list.iter() {
            if t != token {
                pruned.push_back(t);
            }
        }
        env.storage()
            .persistent()
            .set(&crate::storage::DataKey::TokenList, &pruned);

        env.events().publish_event(&TokenRemoved { token });
        Ok(())
    }

    // ------------------------------------------------------------
    // pause / unpause (emergency controls)
    // ------------------------------------------------------------
    /// Access: Admin only
    pub fn pause(env: Env) -> Result<(), ContractError> {
        require_admin(&env)?;

        set_paused(&env, true);
        env.events().publish_event(&ContractPaused {
            timestamp: env.ledger().timestamp(),
        });
        Ok(())
    }

    /// Access: Admin only
    pub fn unpause(env: Env) -> Result<(), ContractError> {
        require_admin(&env)?;

        set_paused(&env, false);
        env.events().publish_event(&ContractUnpaused {
            timestamp: env.ledger().timestamp(),
        });
        Ok(())
    }

    // ------------------------------------------------------------
    // upgrade (Issue #48)
    // ------------------------------------------------------------
    /// Upgrade the contract to a new WASM hash.
    ///
    /// Only the admin can trigger an upgrade. This function emits an event
    /// but does not directly perform the upgrade—that is done by the network
    /// after the contract is authorized to update its code hash via governance.
    ///
    /// # Arguments
    /// - `env`: The Soroban environment
    /// - `new_wasm_hash`: The hash of the new WASM binary to upgrade to (32 bytes)
    ///
    /// # Returns
    /// - `Ok(())` if the upgrade event was successfully published
    /// - `Err(ContractError)` if called by non-admin
    ///
    /// # Notes
    /// This function:
    /// - Requires admin authentication
    /// - Emits a ContractUpgraded event for audit trail
    /// - Does NOT perform the actual upgrade (handled by Soroban runtime)
    /// - Should only be called after off-chain governance approval
    ///
    /// Access: Admin only
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), ContractError> {
        require_admin(&env)?;

        let admin = get_admin(&env).ok_or(ContractError::Unauthorized)?;

        env.events().publish_event(&ContractUpgraded {
            admin,
            new_wasm_hash,
            timestamp: env.ledger().timestamp(),
        });

        Ok(())
    }

    // ------------------------------------------------------------
    // get_contract_stats (read-only view)
    // ------------------------------------------------------------
    /// Access: Anyone
    pub fn get_contract_stats(env: Env) -> ContractStats {
        get_contract_stats(&env)
    }

    // ------------------------------------------------------------
    // list_invoices_by_submitter (Paginated)
    // ------------------------------------------------------------
    /// Access: Anyone
    pub fn list_invoices_by_submitter(
        env: Env,
        submitter: Address,
        page: u32,
        page_size: u32,
    ) -> Vec<Invoice> {
        let page_size = page_size.min(50);
        let invoice_ids = get_submitter_invoices(&env, &submitter);
        let total_invoices = invoice_ids.len();

        let start = page * page_size;
        if start >= total_invoices {
            return Vec::new(&env);
        }

        let end = (start + page_size).min(total_invoices);
        let mut result = Vec::new(&env);

        for i in start..end {
            if let Some(id) = invoice_ids.get(i) {
                result.push_back(load_invoice(&env, id));
            }
        }

        result
    }

    // ------------------------------------------------------------
    // list_invoices_by_lp (Paginated)
    // ------------------------------------------------------------
    /// Access: Anyone
    pub fn list_invoices_by_lp(env: Env, lp: Address, page: u32, page_size: u32) -> Vec<Invoice> {
        let page_size = page_size.min(50);
        let invoice_ids = get_lp_invoices(&env, &lp);
        let total_invoices = invoice_ids.len();

        let start = page * page_size;
        if start >= total_invoices {
            return Vec::new(&env);
        }

        let end = (start + page_size).min(total_invoices);
        let mut result = Vec::new(&env);

        for i in start..end {
            if let Some(id) = invoice_ids.get(i) {
                result.push_back(load_invoice(&env, id));
            }
        }

        result
    }

    // ------------------------------------------------------------
    // submit_invoice (NOW TOKEN-AWARE)
    // ------------------------------------------------------------
    /// Access: Submitter only
    pub fn submit_invoice(
        env: Env,
        freelancer: Address,
        payer: Address,
        amount: i128,
        due_date: u64,
        discount_rate: u32,
        token: Address,
    ) -> Result<u64, ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        require_submitter(&env, &freelancer)?;

        if freelancer == payer {
            return Err(ContractError::SelfInvoice);
        }

        validate_invoice_terms(&env, amount, due_date, discount_rate)?;

        // token validation
        if !is_approved_token(&env, &token) {
            return Err(ContractError::Unauthorized);
        }

        let id = next_invoice_id(&env);

        // Capture the freelancer's reputation score at submission time
        let submitter_reputation = get_payer_score(&env, &freelancer);

        let invoice = Invoice {
            id,
            freelancer: freelancer.clone(),
            payer,
            token,
            amount,
            due_date: due_date.try_into().unwrap(),
            discount_rate,
            status: InvoiceStatus::Pending,
            funder: None,
            funded_at: None,
            amount_funded: 0,
            amount_paid: 0,
            submitter_reputation,
        };

        save_invoice(&env, &invoice);

        // Update submitter index
        add_invoice_to_submitter(&env, &freelancer, id);

        // Increment total invoices counter
        increment_total_invoices(&env);

        env.events().publish_event(&InvoiceSubmitted {
            invoice_id: invoice.id,
            freelancer: invoice.freelancer.clone(),
            payer: invoice.payer.clone(),
            token: invoice.token.clone(),
            amount: invoice.amount,
            due_date: u64::from(invoice.due_date),
            discount_rate: invoice.discount_rate,
            status: invoice.status.clone(),
            timestamp: env.ledger().timestamp(),
        });

        Ok(id)
    }

    // ------------------------------------------------------------
    // update_invoice
    // ------------------------------------------------------------
    /// Access: Submitter only
    pub fn update_invoice(
        env: Env,
        freelancer: Address,
        invoice_id: u64,
        amount: i128,
        due_date: u64,
        discount_rate: u32,
    ) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);
        require_submitter_by_id(&env, &freelancer, invoice_id)?;

        if invoice.status == InvoiceStatus::Pending
            && env.ledger().timestamp() >= u64::from(invoice.due_date)
        {
            invoice.status = InvoiceStatus::Expired;
            save_invoice(&env, &invoice);
            return Err(ContractError::InvoiceExpired);
        }

        match invoice.status {
            InvoiceStatus::Pending => {}
            InvoiceStatus::PartiallyFunded | InvoiceStatus::Funded => {
                return Err(ContractError::AlreadyFunded)
            }
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        validate_invoice_terms(&env, amount, due_date, discount_rate)?;

        invoice.amount = amount;
        invoice.due_date = due_date.try_into().unwrap();
        invoice.discount_rate = discount_rate;

        save_invoice(&env, &invoice);

        env.events().publish_event(&InvoiceUpdated {
            invoice_id: invoice.id,
            freelancer: invoice.freelancer.clone(),
            payer: invoice.payer.clone(),
            token: invoice.token.clone(),
            amount: invoice.amount,
            due_date: u64::from(invoice.due_date),
            discount_rate: invoice.discount_rate,
            status: invoice.status.clone(),
        });

        Ok(())
    }

    // ------------------------------------------------------------
    // submit_invoices_batch
    // ------------------------------------------------------------
    /// Access: Submitter only
    pub fn submit_invoices_batch(
        env: Env,
        invoices: Vec<InvoiceParams>,
    ) -> Result<Vec<u64>, ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        if invoices.len() > 10 {
            return Err(ContractError::BatchTooLarge);
        }

        let mut authenticated_freelancers: Vec<Address> = Vec::new(&env);
        let mut ids = Vec::new(&env);
        for params in invoices.iter() {
            if !authenticated_freelancers.contains(&params.freelancer) {
                require_submitter(&env, &params.freelancer)?;
                authenticated_freelancers.push_back(params.freelancer.clone());
            }

            validate_invoice_terms(&env, params.amount, params.due_date, params.discount_rate)?;

            if !is_approved_token(&env, &params.token) {
                return Err(ContractError::Unauthorized);
            }

            let id = next_invoice_id(&env);

            // Capture the freelancer's reputation score at submission time
            let submitter_reputation = get_payer_score(&env, &params.freelancer);

            let invoice = Invoice {
                id,
                freelancer: params.freelancer.clone(),
                payer: params.payer,
                token: params.token,
                amount: params.amount,
                due_date: params.due_date.try_into().unwrap(),
                discount_rate: params.discount_rate,
                status: InvoiceStatus::Pending,
                funder: None,
                funded_at: None,
                amount_funded: 0,
                amount_paid: 0,
                submitter_reputation,
            };

            save_invoice(&env, &invoice);

            // Update submitter index
            add_invoice_to_submitter(&env, &params.freelancer, id);

            // Increment total invoices counter
            increment_total_invoices(&env);

            env.events().publish_event(&InvoiceSubmitted {
                invoice_id: invoice.id,
                freelancer: invoice.freelancer.clone(),
                payer: invoice.payer.clone(),
                token: invoice.token.clone(),
                amount: invoice.amount,
                due_date: u64::from(invoice.due_date),
                discount_rate: invoice.discount_rate,
                status: invoice.status.clone(),
                timestamp: env.ledger().timestamp(),
            });

            ids.push_back(id);
        }

        Ok(ids)
    }

    // ================================================================
    // Issue #34: LP Priority Queue
    //
    // Design:
    //  1. Any LP calls `join_fund_queue(lp, invoice_id)` to register intent.
    //     Their current LP reputation score is snapshotted.
    //  2. Anyone can call `resolve_fund_queue(invoice_id)` to lock in the
    //     highest-score LP as the approved funder.
    //  3. `fund_invoice` checks: if a QueueResolution exists for this invoice,
    //     only the approved LP may fund it.
    //  If no LP ever joins the queue the existing first-come-first-served
    //  behaviour is preserved unchanged.
    // ================================================================

    /// Register an LP's intent to fund an invoice.
    /// The LP's current reputation score is snapshotted for ordering.
    /// Access: LP only
    pub fn join_fund_queue(env: Env, lp: Address, invoice_id: u64) -> Result<(), ContractError> {
        require_lp(&env, &lp)?;

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        // Queue resolution already happened — too late to join.
        if get_queue_resolution(&env, invoice_id).is_some() {
            return Err(ContractError::NotApprovedFunder);
        }

        let invoice = load_invoice(&env, invoice_id);
        match invoice.status {
            InvoiceStatus::Pending | InvoiceStatus::PartiallyFunded => {}
            InvoiceStatus::Funded => return Err(ContractError::AlreadyFunded),
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        let mut queue = get_fund_queue(&env, invoice_id);

        // Prevent duplicate entries.
        for i in 0..queue.len() {
            if queue.get(i).unwrap().lp == lp {
                return Err(ContractError::AlreadyInQueue);
            }
        }

        let score = get_lp_score(&env, &lp);
        queue.push_back(LpFundRequest {
            lp: lp.clone(),
            score,
        });
        save_fund_queue(&env, invoice_id, &queue);

        env.events().publish_event(&FundRequested {
            invoice_id,
            lp,
            score,
        });

        Ok(())
    }

    /// Select the highest-reputation LP from the queue as the approved funder.
    /// Returns the winning LP address.
    /// Can be called by anyone once at least one LP has joined the queue.
    /// Access: Anyone
    pub fn resolve_fund_queue(env: Env, invoice_id: u64) -> Result<Address, ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        // Already resolved.
        if let Some(approved) = get_queue_resolution(&env, invoice_id) {
            return Ok(approved);
        }

        let queue = get_fund_queue(&env, invoice_id);
        if queue.is_empty() {
            return Err(ContractError::NotFunded); // no one in queue
        }

        // Find the LP with the highest score (ties broken by first-come-first-served).
        let mut best_lp = queue.get(0).unwrap().lp.clone();
        let mut best_score = queue.get(0).unwrap().score;

        for i in 1..queue.len() {
            let entry = queue.get(i).unwrap();
            if entry.score > best_score {
                best_score = entry.score;
                best_lp = entry.lp.clone();
            }
        }

        save_queue_resolution(&env, invoice_id, &best_lp);

        env.events().publish_event(&FundQueueResolved {
            invoice_id,
            approved_lp: best_lp.clone(),
            score: best_score,
        });

        Ok(best_lp)
    }

    // ------------------------------------------------------------
    // fund_invoice (USES invoice.token) — now queue-aware
    // ------------------------------------------------------------
    /// Access: LP only
    pub fn fund_invoice(
        env: Env,
        funder: Address,
        invoice_id: u64,
        fund_amount: i128,
    ) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        require_lp(&env, &funder)?;

        // Issue #71: load the invoice once instead of `invoice_exists` + `load_invoice`
        // (which read the same persistent key twice on the hottest path).
        let mut invoice =
            try_load_invoice(&env, invoice_id).ok_or(ContractError::InvoiceNotFound)?;

        // ── Issue #34: priority queue check ──────────────────────
        // If a queue has been resolved, only the approved LP may fund.
        if let Some(approved) = get_queue_resolution(&env, invoice_id) {
            if approved != funder {
                return Err(ContractError::NotApprovedFunder);
            }
        }

        // Issue #19: the invoice token must still be on the governance allowlist.
        if !is_approved_token(&env, &invoice.token) {
            return Err(ContractError::Unauthorized);
        }

        // Issue #28: reject funding when the payer's reputation is below the
        // configured minimum threshold (default 0 allows everyone).
        let min_payer_reputation = get_min_payer_reputation(&env);
        if min_payer_reputation > 0
            && get_payer_score(&env, &invoice.payer) < min_payer_reputation
        {
            return Err(ContractError::PayerReputationTooLow);
        }

        if invoice.status == InvoiceStatus::Pending
            && env.ledger().timestamp() >= u64::from(invoice.due_date)
        {
            invoice.status = InvoiceStatus::Expired;
            save_invoice(&env, &invoice);
            return Err(ContractError::InvoiceExpired);
        }

        match invoice.status {
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Funded => return Err(ContractError::AlreadyFunded),
            InvoiceStatus::Pending | InvoiceStatus::PartiallyFunded => {} // all good
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        if invoice.amount_funded + fund_amount > invoice.amount {
            return Err(ContractError::OverfundingRejected);
        }

        // --- Execute transfer ---
        let token = token_client(&env, &invoice.token);
        let contract_address = env.current_contract_address();

        // Handle XLM precision if needed (SAC wrapper handles conversion internally)
        let normalized_fund_amount = if is_xlm_token(&env, &invoice.token) {
            normalize_xlm_amount(fund_amount)
        } else {
            normalize_usdc_amount(fund_amount)
        };

        let fund_discount = normalized_fund_amount
            .checked_mul(discount_rate_as_i128(invoice.discount_rate))
            .unwrap_or(0)
            / 10_000;
        let cost = normalized_fund_amount - fund_discount;

        token.transfer(&funder, &contract_address, &cost);

        // --- Update contributor list ---
        let mut funders = get_invoice_funders(&env, invoice_id);
        let mut found = false;
        for i in 0..funders.len() {
            let (addr, amt) = funders.get(i).unwrap();
            if addr == funder {
                funders.set(i, (addr, amt + fund_amount));
                found = true;
                break;
            }
        }
        if !found {
            funders.push_back((funder.clone(), fund_amount));
        }
        save_invoice_funders(&env, invoice_id, &funders);

        // --- Update invoice state ---
        invoice.amount_funded += fund_amount;

        if invoice.amount_funded == invoice.amount {
            // Fully funded — pay out to freelancer
            let discount_amount = invoice
                .amount
                .checked_mul(discount_rate_as_i128(invoice.discount_rate))
                .unwrap_or(0)
                / 10_000;
            let freelancer_payout = invoice.amount - discount_amount;

            token.transfer(&contract_address, &invoice.freelancer, &freelancer_payout);

            invoice.status = InvoiceStatus::Funded;
            invoice.funded_at = Some(env.ledger().timestamp().try_into().unwrap());
            invoice.funder = Some(funder.clone());

            // Boost LP score on successful funding
            let current_lp_score = get_lp_score(&env, &funder);
            set_lp_score(&env, &funder, current_lp_score + 1);
        } else {
            invoice.status = InvoiceStatus::PartiallyFunded;
        }

        save_invoice(&env, &invoice);

        // Update LP index
        add_invoice_to_lp(&env, &funder, invoice_id);

        // Increment total funded counter if fully funded
        if invoice.status == InvoiceStatus::Funded {
            increment_total_funded(&env);
        }

        add_volume(&env, &invoice.token, fund_amount);

        notify_distribution_funding(&env, &funder, fund_amount);

        let now = env.ledger().timestamp();

        let seconds_to_due = if u64::from(invoice.due_date) > now {
            u64::from(invoice.due_date) - now
        } else {
            0
        };

        let days_to_due = seconds_to_due / (24 * 60 * 60);

        let effective_yield_bps = ((invoice.discount_rate as u64 * days_to_due) / 365) as u32;

        env.events().publish_event(&InvoiceFunded {
            invoice_id: invoice.id,
            funder: funder.clone(),
            freelancer: invoice.freelancer.clone(),
            payer: invoice.payer.clone(),
            token: invoice.token.clone(),
            fund_amount,
            amount_funded: invoice.amount_funded,
            invoice_amount: invoice.amount,
            due_date: u64::from(invoice.due_date),
            discount_rate: invoice.discount_rate,
            funded_at: invoice.funded_at.map(|ts| ts.into()),
            status: invoice.status.clone(),

            // NEW
            lp: funder.clone(),
            effective_yield_bps,
            timestamp: now,
        });

        Ok(())
    }

    // ------------------------------------------------------------
    // transfer_invoice
    // ------------------------------------------------------------
    /// Access: Submitter only
    pub fn transfer_invoice(
        env: Env,
        invoice_id: u64,
        new_freelancer: Address,
    ) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        require_submitter_by_id(&env, &invoice.freelancer, invoice_id)?;

        match invoice.status {
            InvoiceStatus::Pending => {}
            InvoiceStatus::PartiallyFunded | InvoiceStatus::Funded => {
                return Err(ContractError::AlreadyFunded)
            }
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        let old_freelancer = invoice.freelancer.clone();
        invoice.freelancer = new_freelancer.clone();

        save_invoice(&env, &invoice);

        // Update submitter index
        remove_invoice_from_submitter(&env, &old_freelancer, invoice_id);
        add_invoice_to_submitter(&env, &new_freelancer, invoice_id);

        env.events().publish_event(&InvoiceTransferred {
            invoice_id,
            old_freelancer,
            new_freelancer,
            status: invoice.status.clone(),
        });

        Ok(())
    }

    // ------------------------------------------------------------
    // cancel_invoice
    // ------------------------------------------------------------
    /// Access: Submitter only
    pub fn cancel_invoice(env: Env, invoice_id: u64) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        require_submitter_by_id(&env, &invoice.freelancer, invoice_id)?;

        match invoice.status {
            InvoiceStatus::Pending => {}
            InvoiceStatus::PartiallyFunded => {
                let funders = get_invoice_funders(&env, invoice_id);
                let token = token_client(&env, &invoice.token);
                let contract_address = env.current_contract_address();
                for i in 0..funders.len() {
                    let (funder_addr, fund_amt) = funders.get(i).unwrap();
                    let fund_discount = fund_amt
                        .checked_mul(discount_rate_as_i128(invoice.discount_rate))
                        .unwrap_or(0)
                        / 10_000;
                    let refund = fund_amt - fund_discount;
                    token.transfer(&contract_address, &funder_addr, &refund);
                }
            }
            InvoiceStatus::Funded => return Err(ContractError::AlreadyFunded),
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        invoice.status = InvoiceStatus::Cancelled;

        save_invoice(&env, &invoice);

        env.events().publish_event(&InvoiceCancelled {
            invoice_id,
            freelancer: invoice.freelancer.clone(),
            status: invoice.status.clone(),
        });

        Ok(())
    }

    // ------------------------------------------------------------
    // expire_invoice
    // ------------------------------------------------------------
    /// Access: Anyone
    pub fn expire_invoice(env: Env, invoice_id: u64) -> Result<(), ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        if env.ledger().timestamp() < u64::from(invoice.due_date) {
            return Err(ContractError::NotYetDefaulted);
        }

        match invoice.status {
            InvoiceStatus::Pending => {
                invoice.status = InvoiceStatus::Expired;
                save_invoice(&env, &invoice);
                Ok(())
            }
            InvoiceStatus::PartiallyFunded | InvoiceStatus::Funded => {
                Err(ContractError::AlreadyFunded)
            }
            InvoiceStatus::Paid => Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => Err(ContractError::AlreadyCancelled),
        }
    }

    // ------------------------------------------------------------
    // mark_paid (USES invoice.token)
    // ------------------------------------------------------------
    /// Access: Payer only
    pub fn mark_paid(env: Env, invoice_id: u64, amount: i128) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        // Issue #71: single load instead of `invoice_exists` + `load_invoice`.
        let mut invoice =
            try_load_invoice(&env, invoice_id).ok_or(ContractError::InvoiceNotFound)?;

        require_payer_by_id(&env, invoice_id)?;

        match invoice.status {
            InvoiceStatus::Pending | InvoiceStatus::PartiallyFunded => {
                return Err(ContractError::NotFunded)
            }
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Funded => {}
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        let remaining = invoice.amount - invoice.amount_paid;
        if amount > remaining {
            return Err(ContractError::OverpaymentRejected);
        }

        let funders = get_invoice_funders(&env, invoice_id);
        if funders.len() == 0 {
            return Err(ContractError::NotFunded);
        }

        let token = token_client(&env, &invoice.token);
        let contract_address = env.current_contract_address();

        // Handle XLM precision if needed (SAC wrapper handles conversion internally)
        let normalized_amount = if is_xlm_token(&env, &invoice.token) {
            normalize_xlm_amount(amount)
        } else {
            normalize_usdc_amount(amount)
        };

        // Payer sends partial/full amount to the contract
        token.transfer(&invoice.payer, &contract_address, &normalized_amount);

        invoice.amount_paid += amount;

        // If not fully paid, save and emit partial event
        if invoice.amount_paid < invoice.amount {
            save_invoice(&env, &invoice);
            env.events().publish_event(&InvoicePartiallyPaid {
                invoice_id: invoice.id,
                payer: invoice.payer.clone(),
                amount_paid_now: amount,
                total_amount_paid: invoice.amount_paid,
                remaining_amount: invoice.amount - invoice.amount_paid,
            });
            return Ok(());
        }

        // --- FULL PAYMENT LOGIC ---
        // Calculate protocol fee and deduct it
        let fee_rate: u32 = env
            .storage()
            .instance()
            .get(&crate::storage::DataKey::FeeRate)
            .unwrap_or(0);
        let protocol_fee = invoice.amount.checked_mul(fee_rate as i128).unwrap_or(0) / 10_000;

        if protocol_fee > 0 {
            let admin: Address = env
                .storage()
                .instance()
                .get(&crate::storage::DataKey::Admin)
                .unwrap();
            token.transfer(&contract_address, &admin, &protocol_fee);
        }

        let distribute_amount = invoice.amount - protocol_fee;

        // Legacy compatibility: use first LP for event emission
        let primary_lp = funders.get(0).unwrap().0.clone();

        // Total amount funded by primary LP
        let primary_lp_funded = funders.get(0).unwrap().1;

        // LP payout after settlement distribution
        let primary_lp_payout = distribute_amount
            .checked_mul(primary_lp_funded)
            .unwrap_or(0)
            / invoice.amount;

        // LP earnings
        let lp_earned = primary_lp_payout - primary_lp_funded;

        // Distribute proportionally to funders
        for i in 0..funders.len() {
            let (funder_addr, fund_amt) = funders.get(i).unwrap();
            let funder_share =
                distribute_amount.checked_mul(fund_amt).unwrap_or(0) / invoice.amount;
            if funder_share > 0 {
                token.transfer(&contract_address, &funder_addr, &funder_share);
            }
        }

        // ---- Update invoice ----
        invoice.status = InvoiceStatus::Paid;

        save_invoice(&env, &invoice);

        // Increment total paid counter
        increment_total_paid(&env);

        let paid_on_time = env.ledger().timestamp() <= u64::from(invoice.due_date);
        notify_distribution_settlement(&env, &invoice.freelancer, &invoice.payer, paid_on_time);

        // --- Update payer reputation ---
        let current_score = get_payer_score(&env, &invoice.payer);
        set_payer_score(&env, &invoice.payer, current_score + 1);

        env.events().publish_event(&InvoicePaid {
            invoice_id: invoice.id,
            payer: invoice.payer.clone(),
            lp: primary_lp,
            freelancer: invoice.freelancer.clone(),
            token: invoice.token.clone(),
            amount_paid: invoice.amount,
            lp_earned,
            lp_payout: primary_lp_payout,
            settlement_timestamp: env.ledger().timestamp(),
            paid_on_time,
            status: invoice.status.clone(),
        });

        Ok(())
    }

    // ----------------------------------------------------------------
    // claim_yield
    // ----------------------------------------------------------------
    /// Access: LP only
    pub fn claim_yield(env: Env, invoice_id: u64) -> Result<i128, ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let invoice = load_invoice(&env, invoice_id);

        // Only the funder can query their own yield
        if let Some(ref funder) = invoice.funder {
            require_lp_by_id(&env, funder, invoice_id)?;
        } else {
            return Err(ContractError::NothingToClaim);
        }

        match invoice.status {
            InvoiceStatus::Pending | InvoiceStatus::PartiallyFunded | InvoiceStatus::Funded => {
                Ok(0)
            }
            InvoiceStatus::Defaulted => Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => Err(ContractError::AlreadyCancelled),
            InvoiceStatus::Paid => {
                let yield_amount = invoice
                    .amount
                    .checked_mul(discount_rate_as_i128(invoice.discount_rate))
                    .unwrap_or(0)
                    / 10_000;
                Ok(yield_amount)
            }
        }
    }

    // ----------------------------------------------------------------
    // claim_default
    // ----------------------------------------------------------------
    /// Access: LP only
    pub fn claim_default(env: Env, funder: Address, invoice_id: u64) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        require_lp(&env, &funder)?;

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        let funders = get_invoice_funders(&env, invoice_id);
        let mut is_funder = false;
        for i in 0..funders.len() {
            if funders.get(i).unwrap().0 == funder {
                is_funder = true;
                break;
            }
        }

        if !is_funder {
            return Err(ContractError::Unauthorized);
        }

        let now = env.ledger().timestamp();
        if now < u64::from(invoice.due_date) {
            return Err(ContractError::NotYetDefaulted);
        }

        match invoice.status {
            InvoiceStatus::Funded => {}
            InvoiceStatus::Pending | InvoiceStatus::PartiallyFunded => {
                return Err(ContractError::NotFunded)
            }
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Disputed => return Err(ContractError::InvoiceDisputed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
        }

        let token = token_client(&env, &invoice.token);
        let contract_address = env.current_contract_address();

        let mut total_refunded = 0;

        for i in 0..funders.len() {
            let (funder_addr, fund_amt) = funders.get(i).unwrap();
            let fund_discount = fund_amt
                .checked_mul(discount_rate_as_i128(invoice.discount_rate))
                .unwrap_or(0)
                / 10_000;
            let refund = fund_amt - fund_discount;
            token.transfer(&contract_address, &funder_addr, &refund);
            total_refunded += refund;
        }

        invoice.status = InvoiceStatus::Defaulted;
        save_invoice(&env, &invoice);

        // --- Update payer reputation ---
        // Snapshot the score BEFORE applying the penalty so appeal_default()
        // can restore it exactly if the appeal is upheld.
        let current_score = get_payer_score(&env, &invoice.payer);
        save_pre_default_payer_score(&env, invoice_id, current_score);

        if current_score > 5 {
            set_payer_score(&env, &invoice.payer, current_score - 5);
        } else {
            set_payer_score(&env, &invoice.payer, 0);
        }

        env.events().publish_event(&InvoiceDefaulted {
            invoice_id: invoice.id,
            funder,
            freelancer: invoice.freelancer.clone(),
            payer: invoice.payer.clone(),
            token: invoice.token.clone(),
            amount: invoice.amount,
            due_date: u64::from(invoice.due_date),
            defaulted_at: now,
            discount_amount: total_refunded,
            status: invoice.status.clone(),
        });

        Ok(())
    }

    // ================================================================
    // Issue #36: appeal_default — payer contests an unfair default
    //
    // Flow:
    //   1. Payer calls `appeal_default(invoice_id, evidence_hash)`.
    //   2. Invoice transitions to `Appealed` status.
    //   3. Admin/governance calls `resolve_appeal(invoice_id, upheld)`.
    //      - upheld=true  → default reversed, score restored.
    //      - upheld=false → invoice remains Defaulted.
    // ================================================================

    /// File an appeal against an unfair default marking.
    ///
    /// * `invoice_id`    – the defaulted invoice
    /// * `evidence_hash` – SHA-256 hash of off-chain evidence provided by the payer
    /// Access: Payer only
    pub fn appeal_default(
        env: Env,
        invoice_id: u64,
        evidence_hash: BytesN<32>,
    ) -> Result<(), ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        // Only the payer may appeal.
        require_payer_by_id(&env, invoice_id)?;

        // Check AlreadyAppealed BEFORE status check: after the first appeal the
        // status is `Appealed` (not `Defaulted`), so the status guard would fire
        // with the wrong error code if checked first.
        if get_appeal(&env, invoice_id).is_some() {
            return Err(ContractError::AlreadyAppealed);
        }

        // Invoice must be in Defaulted state.
        if invoice.status != InvoiceStatus::Defaulted {
            return Err(ContractError::NotDefaulted);
        }

        let now = env.ledger().timestamp();

        // Appeal must be filed within the appeal window after default.
        // A default can only occur after due_date, so we measure from due_date.
        if now > u64::from(invoice.due_date) + APPEAL_WINDOW_SECONDS {
            return Err(ContractError::AppealWindowClosed);
        }

        // Use the pre-default score snapshot saved by claim_default().
        // Fall back to the current score if somehow missing (shouldn't happen).
        let pre_default_score = get_pre_default_payer_score(&env, invoice_id)
            .unwrap_or_else(|| get_payer_score(&env, &invoice.payer));

        save_appeal(
            &env,
            invoice_id,
            &AppealRecord {
                evidence_hash: evidence_hash.clone(),
                appealed_at: now,
                pre_default_score,
            },
        );

        invoice.status = InvoiceStatus::Appealed;
        save_invoice(&env, &invoice);

        env.events().publish_event(&DefaultAppealed {
            invoice_id,
            payer: invoice.payer.clone(),
            evidence_hash,
            appealed_at: now,
        });

        Ok(())
    }

    /// Resolve a pending appeal (admin / governance only).
    ///
    /// * `upheld=true`  → reverse the default, restore pre-default score, status → Defaulted (reversed).
    ///   In practice the status transitions back to Defaulted with score restored so the LP
    ///   can still collect principal they were already refunded. The key effect is reputation repair.
    /// * `upheld=false` → reject the appeal; invoice remains Defaulted (status reverts from Appealed).
    /// Access: Admin only
    pub fn resolve_appeal(env: Env, invoice_id: u64, upheld: bool) -> Result<(), ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        if invoice.status != InvoiceStatus::Appealed {
            return Err(ContractError::NotDefaulted);
        }

        let appeal = get_appeal(&env, invoice_id).ok_or(ContractError::InvoiceNotFound)?;

        let now = env.ledger().timestamp();

        if upheld {
            // Restore the payer's reputation to what it was before the default.
            set_payer_score(&env, &invoice.payer, appeal.pre_default_score);
            // Status moves back to Defaulted — the LP still received their refund,
            // but the reputational penalty on the payer is reversed.
            invoice.status = InvoiceStatus::Defaulted;
        } else {
            // Appeal rejected; mark as Defaulted again (was temporarily Appealed).
            invoice.status = InvoiceStatus::Defaulted;
        }

        save_invoice(&env, &invoice);

        env.events().publish_event(&AppealResolved {
            invoice_id,
            payer: invoice.payer.clone(),
            upheld,
            resolved_at: now,
        });

        Ok(())
    }

    // ================================================================
    // Dispute Mechanism — payer raised disputes before settlement
    // ================================================================

    /// Dispute an invoice before settlement.
    ///
    /// * `invoice_id`  – the invoice to dispute
    /// * `reason_hash` – SHA-256 hash of off-chain dispute evidence
    /// Access: Payer only
    pub fn dispute_invoice(
        env: Env,
        invoice_id: u64,
        reason_hash: BytesN<32>,
    ) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        // Only the payer may dispute.
        require_payer_by_id(&env, invoice_id)?;

        // Check if already disputed.
        if get_dispute(&env, invoice_id).is_some() {
            return Err(ContractError::AlreadyDisputed);
        }

        // Only Pending, PartiallyFunded or Funded invoices can be disputed (before settlement).
        match invoice.status {
            InvoiceStatus::Pending | InvoiceStatus::PartiallyFunded | InvoiceStatus::Funded => {}
            InvoiceStatus::Paid => return Err(ContractError::AlreadyPaid),
            InvoiceStatus::Defaulted => return Err(ContractError::InvoiceDefaulted),
            InvoiceStatus::Appealed => return Err(ContractError::InvoiceAppealed),
            InvoiceStatus::Expired => return Err(ContractError::InvoiceExpired),
            InvoiceStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
            InvoiceStatus::Disputed => return Err(ContractError::AlreadyDisputed),
        }

        let now_ts = env.ledger().timestamp();
        let now_ledger = env.ledger().sequence();

        save_dispute(
            &env,
            invoice_id,
            &DisputeRecord {
                reason_hash: reason_hash.clone(),
                disputed_at: now_ledger.into(),
            },
        );

        invoice.status = InvoiceStatus::Disputed;
        save_invoice(&env, &invoice);

        env.events().publish_event(&InvoiceDisputed {
            invoice_id,
            payer: invoice.payer.clone(),
            reason_hash,
            disputed_at: now_ts,
        });

        Ok(())
    }

    /// Resolve a dispute (admin / governance only).
    ///
    /// * `resolution_hash` – Optional hash of resolution details
    /// * `resolution`      – Ruling: 1 = Upheld (Payer right), 2 = Rejected (Freelancer right)
    /// Access: Admin only
    pub fn resolve_dispute(
        env: Env,
        invoice_id: u64,
        resolution_hash: BytesN<32>,
        resolution: u32,
    ) -> Result<(), ContractError> {
        require_admin(&env)?;

        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        if invoice.status != InvoiceStatus::Disputed {
            return Err(ContractError::NotDisputed);
        }

        match resolution {
            1 => {
                // Upheld: Payer is right.
                // Refund LPs if it was funded.
                let funders = get_invoice_funders(&env, invoice_id);
                if !funders.is_empty() {
                    let token = token_client(&env, &invoice.token);
                    let contract_address = env.current_contract_address();
                    for i in 0..funders.len() {
                        let (funder_addr, fund_amt) = funders.get(i).unwrap();
                        let fund_discount = fund_amt
                            .checked_mul(discount_rate_as_i128(invoice.discount_rate))
                            .unwrap_or(0)
                            / 10_000;
                        let refund = fund_amt - fund_discount;
                        token.transfer(&contract_address, &funder_addr, &refund);
                    }
                }
                invoice.status = InvoiceStatus::Cancelled;
            }
            2 => {
                // Rejected: Freelancer is right.
                // Restore status based on funding level.
                if invoice.amount_funded == invoice.amount {
                    invoice.status = InvoiceStatus::Funded;
                } else if invoice.amount_funded > 0 {
                    invoice.status = InvoiceStatus::PartiallyFunded;
                } else {
                    invoice.status = InvoiceStatus::Pending;
                }
            }
            _ => return Err(ContractError::Unauthorized), // Invalid resolution
        }

        save_invoice(&env, &invoice);

        env.events().publish_event(&DisputeResolved {
            invoice_id,
            resolution_hash,
            resolution,
            resolved_at: env.ledger().timestamp(),
        });

        Ok(())
    }

    /// Auto-resolve a dispute after the timeout has passed.
    ///
    /// * `invoice_id` – the invoice to auto-resolve
    /// Access: Anyone
    pub fn auto_resolve_dispute(env: Env, invoice_id: u64) -> Result<(), ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }

        let mut invoice = load_invoice(&env, invoice_id);

        if invoice.status != InvoiceStatus::Disputed {
            return Err(ContractError::NotDisputed);
        }

        let dispute = get_dispute(&env, invoice_id).ok_or(ContractError::InvoiceNotFound)?;
        let config = crate::storage::get_config(&env).ok_or(ContractError::Unauthorized)?;

        let now_ledger = env.ledger().sequence();

        if u64::from(now_ledger) < dispute.disputed_at + config.dispute_timeout_ledgers {
            return Err(ContractError::Unauthorized); // Or a more specific error like TimeoutNotReached
        }

        // Auto-resolve: Default to Rejected (Freelancer right) to prevent DOS.
        if invoice.amount_funded == invoice.amount {
            invoice.status = InvoiceStatus::Funded;
        } else if invoice.amount_funded > 0 {
            invoice.status = InvoiceStatus::PartiallyFunded;
        } else {
            invoice.status = InvoiceStatus::Pending;
        }

        save_invoice(&env, &invoice);

        env.events().publish_event(&DisputeResolved {
            invoice_id,
            resolution_hash: BytesN::from_array(&env, &[0u8; 32]),
            resolution: 2, // Rejected
            resolved_at: env.ledger().timestamp(),
        });

        Ok(())
    }

    // ================================================================
    // Contract Configuration
    // ================================================================

    pub fn update_config(
        env: Env,
        caller: Address,
        high_rep_threshold: u32,
        bonus_bps: u32,
        min_discount_rate_bps: u32,
        decay_rate_bps: u32,
        decay_period_ledgers: u64,
        dispute_timeout_ledgers: u64,
        xlm_sac_address: Address,
    ) -> Result<(), ContractError> {
        crate::config::update_config(
            &env,
            &caller,
            high_rep_threshold,
            bonus_bps,
            min_discount_rate_bps,
            decay_rate_bps,
            decay_period_ledgers,
            dispute_timeout_ledgers,
            xlm_sac_address,
        )
        .map_err(|_| ContractError::Unauthorized)
    }

    pub fn get_config(env: Env) -> Result<Config, ContractError> {
        crate::storage::get_config(&env).ok_or(ContractError::Unauthorized)
    }
    // payer_score
    // ----------------------------------------------------------------
    /// Access: Anyone
    pub fn payer_score(env: Env, payer: Address) -> u32 {
        get_payer_score(&env, &payer)
    }

    // ----------------------------------------------------------------
    // lp_score  (Issue #34)
    // ----------------------------------------------------------------
    /// Access: Anyone
    pub fn lp_score(env: Env, lp: Address) -> u32 {
        get_lp_score(&env, &lp)
    }

    // ----------------------------------------------------------------
    // get_reputation (Issue #26)
    // ----------------------------------------------------------------
    /// Read an address's detailed reputation profile. Unknown addresses return
    /// a zeroed profile rather than panicking.
    /// Access: Anyone
    pub fn get_reputation(env: Env, address: Address) -> ReputationProfile {
        get_reputation(&env, &address)
    }

    // ----------------------------------------------------------------
    // min_payer_reputation config (Issue #28)
    // ----------------------------------------------------------------
    /// Current minimum payer reputation required to fund an invoice (0 = off).
    /// Access: Anyone
    pub fn min_payer_reputation(env: Env) -> u32 {
        get_min_payer_reputation(&env)
    }

    /// Update the minimum payer reputation threshold.
    /// Access: Admin only
    pub fn set_min_payer_reputation(env: Env, value: u32) -> Result<(), ContractError> {
        require_admin(&env)?;
        let updated_by = get_admin(&env).ok_or(ContractError::Unauthorized)?;
        let old_value = get_min_payer_reputation(&env);
        set_min_payer_reputation(&env, value);
        env.events().publish_event(&ParameterUpdated {
            param_name: Symbol::new(&env, "min_payer_reputation"),
            old_value: old_value as i128,
            new_value: value as i128,
            updated_by,
        });
        Ok(())
    }

    // ----------------------------------------------------------------
    // suggested_discount_rate
    // ----------------------------------------------------------------
    /// Access: Anyone
    pub fn suggested_discount_rate(env: Env, payer: Address) -> u32 {
        let score = get_payer_score(&env, &payer);
        let capped = score.min(100);
        let rate = 500 + (100 - capped) * 5;
        rate.max(50)
    }

    /// Returns the invoice with the given `invoice_id`.
    ///
    /// This is a read-only view method that returns the full `Invoice`
    /// struct, including submitter, payer, LP, token, amount, discount rate,
    /// due date, status, and funding state.
    ///
    /// # Errors
    ///
    /// Returns `ContractError::InvoiceNotFound` if the invoice does not exist.
    // ----------------------------------------------------------------
    // get_invoice
    // ----------------------------------------------------------------
    /// Access: Anyone
    pub fn get_invoice(env: Env, invoice_id: u64) -> Result<Invoice, ContractError> {
        if !invoice_exists(&env, invoice_id) {
            return Err(ContractError::InvoiceNotFound);
        }
        Ok(load_invoice(&env, invoice_id))
    }

    /// Access: Anyone
    pub fn get_invoice_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get(&crate::storage::DataKey::InvoiceCount)
            .unwrap_or(0)
    }
}

// ----------------------------------------------------------------
// TOKEN HELPERS
// ----------------------------------------------------------------

fn token_client<'a>(env: &'a Env, token: &Address) -> TokenClient<'a> {
    TokenClient::new(env, token)
}

fn discount_rate_as_i128(rate: u32) -> i128 {
    rate as i128
}

// ----------------------------------------------------------------
// XLM PRECISION HANDLING
// ----------------------------------------------------------------
// XLM has 7 decimal places (1 XLM = 10,000,000 stroops)
// USDC has 6 decimal places (1 USDC = 1,000,000 units)
// These helpers ensure correct precision handling

const XLM_DECIMALS: u32 = 7;
const USDC_DECIMALS: u32 = 6;

/// Check if a token address is the XLM SAC address
fn is_xlm_token(env: &Env, token: &Address) -> bool {
    if let Some(config) = crate::storage::get_config(env) {
        token == &config.xlm_sac_address
    } else {
        false
    }
}

/// Convert amount from XLM precision (7 decimals) to contract precision
/// This is a no-op for now since we store amounts in their native token precision,
/// but provides a hook for future precision normalization if needed
fn normalize_xlm_amount(amount: i128) -> i128 {
    amount
}

/// Convert amount from USDC precision (6 decimals) to contract precision
/// This is a no-op for now since we store amounts in their native token precision,
/// but provides a hook for future precision normalization if needed
fn normalize_usdc_amount(amount: i128) -> i128 {
    amount
}

fn validate_invoice_terms(
    env: &Env,
    amount: i128,
    due_date: u64,
    discount_rate: u32,
) -> Result<(), ContractError> {
    if amount < 1_000_000 {
        return Err(ContractError::InvalidAmount);
    }

    let max_rate: u32 = env
        .storage()
        .instance()
        .get(&crate::storage::DataKey::MaxDiscountRate)
        .unwrap_or(5000);
    if discount_rate == 0 || discount_rate > max_rate {
        return Err(ContractError::InvalidDiscountRate);
    }

    // The on-chain storage representation now uses u32 timestamps.
    if due_date > u64::from(u32::MAX) {
        return Err(ContractError::InvalidDueDate);
    }

    let now = env.ledger().timestamp();

    // Validate due date is in the future
    if due_date <= now {
        return Err(ContractError::InvalidDueDate);
    }

    if due_date < now + MIN_INVOICE_DURATION {
        return Err(ContractError::DueDateTooSoon);
    }

    if due_date > now + MAX_INVOICE_DURATION {
        return Err(ContractError::DueDateTooFar);
    }

    Ok(())
}

fn is_approved_token(env: &Env, token: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::ApprovedToken(token.clone()))
        .unwrap_or(false)
}

fn notify_distribution_funding(env: &Env, lp: &Address, amount_usdc_equivalent: i128) {
    let Some(dist_contract) = env
        .storage()
        .instance()
        .get::<_, Address>(&crate::storage::DataKey::DistributionContract)
    else {
        return;
    };

    let args = vec![
        env,
        lp.clone().into_val(env),
        amount_usdc_equivalent.into_val(env),
    ];
    env.invoke_contract::<()>(&dist_contract, &Symbol::new(env, "accrue_lp"), args);
}

fn notify_distribution_settlement(
    env: &Env,
    freelancer: &Address,
    payer: &Address,
    settled_on_time: bool,
) {
    let Some(dist_contract) = env
        .storage()
        .instance()
        .get::<_, Address>(&crate::storage::DataKey::DistributionContract)
    else {
        return;
    };

    let args = vec![
        env,
        freelancer.clone().into_val(env),
        payer.clone().into_val(env),
        settled_on_time.into_val(env),
    ];
    env.invoke_contract::<()>(&dist_contract, &Symbol::new(env, "accrue_settlement"), args);
}

// ----------------------------------------------------------------
// TEST MODULES
// ----------------------------------------------------------------

mod test;
#[cfg(test)]
mod tests_access_control;
#[cfg(test)]
mod tests_governance_features;
mod tests_appeal;
mod tests_arithmetic;
mod tests_auth;
mod tests_dispute;
mod tests_distribution;
mod tests_invariants;
#[cfg(test)]
mod tests_invoice_paid_event;
#[cfg(test)]
mod tests_lp_funding_details_event;
mod tests_lp_priority_queue;
mod tests_mutation;
#[cfg(test)]
mod tests_partial_payment;
mod tests_protocol_fee;
mod tests_security;
mod tests_state_machine;
mod tests_storage;
mod tests_storage_extra;
