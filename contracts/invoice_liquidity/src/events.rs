use soroban_sdk::{contractevent, Address, BytesN, Symbol};

use crate::invoice::InvoiceStatus;

/// Emitted when governance adds a token to the funding allowlist (Issue #19).
#[contractevent(topics = ["token_added"])]
#[derive(Clone, Debug, PartialEq)]
pub struct TokenAdded {
    #[topic]
    pub token: Address,
}

/// Emitted when governance removes a token from the funding allowlist (Issue #19).
#[contractevent(topics = ["token_removed"])]
#[derive(Clone, Debug, PartialEq)]
pub struct TokenRemoved {
    #[topic]
    pub token: Address,
}

#[contractevent(topics = ["submitted"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceSubmitted {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub freelancer: Address,
    #[topic]
    pub payer: Address,
    pub token: Address,
    pub amount: i128,
    pub due_date: u64,
    pub discount_rate: u32,
    pub status: InvoiceStatus,
    /// Ledger timestamp when the invoice was submitted.  Included so indexers
    /// can reconstruct the full invoice record from events alone.
    pub timestamp: u64,
}

#[contractevent(topics = ["updated"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceUpdated {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub freelancer: Address,
    #[topic]
    pub payer: Address,
    pub token: Address,
    pub amount: i128,
    pub due_date: u64,
    pub discount_rate: u32,
    pub status: InvoiceStatus,
}

#[contractevent(topics = ["funded"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceFunded {
    #[topic]
    pub invoice_id: u64,

    #[topic]
    pub funder: Address,

    pub freelancer: Address,
    pub payer: Address,
    pub token: Address,

    pub fund_amount: i128,
    pub amount_funded: i128,
    pub invoice_amount: i128,

    pub due_date: u64,
    pub discount_rate: u32,

    pub funded_at: Option<u64>,
    pub status: InvoiceStatus,

    // NEW FIELDS
    pub lp: Address,
    pub effective_yield_bps: u32,
    pub timestamp: u64,
}

#[contractevent(topics = ["paid"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoicePaid {
    #[topic]
    pub invoice_id: u64,

    #[topic]
    pub payer: Address,

    #[topic]
    pub lp: Address,

    pub freelancer: Address,
    pub token: Address,

    /// Full amount settled by payer
    pub amount_paid: i128,

    /// LP earnings = amount_paid - amount_funded
    pub lp_earned: i128,

    /// Total amount distributed to LP
    pub lp_payout: i128,

    /// Settlement ledger timestamp
    pub settlement_timestamp: u64,

    pub paid_on_time: bool,
    pub status: InvoiceStatus,
}

#[contractevent(topics = ["partially_paid"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoicePartiallyPaid {
    #[topic]
    pub invoice_id: u64,

    #[topic]
    pub payer: Address,

    pub amount_paid_now: i128,
    pub total_amount_paid: i128,
    pub remaining_amount: i128,
}

#[contractevent(topics = ["paused"])]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractPaused {
    pub timestamp: u64,
}

#[contractevent(topics = ["unpaused"])]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractUnpaused {
    pub timestamp: u64,
}

#[contractevent(topics = ["defaulted"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceDefaulted {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub funder: Address,
    pub freelancer: Address,
    pub payer: Address,
    pub token: Address,
    pub amount: i128,
    pub due_date: u64,
    pub defaulted_at: u64,
    pub discount_amount: i128,
    pub status: InvoiceStatus,
}

#[contractevent(topics = ["transferred"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceTransferred {
    #[topic]
    pub invoice_id: u64,
    pub old_freelancer: Address,
    pub new_freelancer: Address,
    pub status: InvoiceStatus,
}

#[contractevent(topics = ["cancelled"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceCancelled {
    #[topic]
    pub invoice_id: u64,
    pub freelancer: Address,
    pub status: InvoiceStatus,
}

/// Emitted whenever the contract admin address is updated.
/// Provides a permanent on-chain audit trail for admin transitions.
#[contractevent(topics = ["admin_changed"])]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminChanged {
    pub old_admin: Address,
    pub new_admin: Address,
    /// Ledger timestamp of the change.
    pub timestamp: u64,
}

/// Emitted whenever a governance-controlled numeric parameter changes.
///
/// The `param_name` topic is a stable audit identifier. Keep these strings
/// unique per parameter so off-chain indexers can reconstruct config history.
#[contractevent(topics = ["parameter_updated"])]
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterUpdated {
    #[topic]
    pub param_name: Symbol,
    pub old_value: i128,
    pub new_value: i128,
    #[topic]
    pub updated_by: Address,
}

#[contractevent(topics = ["upgraded"])]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub new_wasm_hash: BytesN<32>,
    pub timestamp: u64,
}

// ── Issue #36: appeal_default events ──────────────────────────────────────────

/// Emitted when a payer files an appeal against an unfair default marking.
#[contractevent(topics = ["default_appealed"])]
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultAppealed {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub payer: Address,
    /// SHA-256 hash of off-chain evidence provided by the payer.
    pub evidence_hash: BytesN<32>,
    pub appealed_at: u64,
}

/// Emitted when governance resolves a payer's appeal.
#[contractevent(topics = ["appeal_resolved"])]
#[derive(Clone, Debug, PartialEq)]
pub struct AppealResolved {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub payer: Address,
    /// true = appeal upheld (default reversed); false = appeal rejected.
    pub upheld: bool,
    pub resolved_at: u64,
}

// ── Dispute events ──────────────────────────────────────────────────────────

/// Emitted when a payer disputes an invoice before settlement.
#[contractevent(topics = ["disputed"])]
#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceDisputed {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub payer: Address,
    /// SHA-256 hash of off-chain dispute evidence.
    pub reason_hash: BytesN<32>,
    pub disputed_at: u64,
}

/// Emitted when governance resolves a dispute.
#[contractevent(topics = ["dispute_resolved"])]
#[derive(Clone, Debug, PartialEq)]
pub struct DisputeResolved {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub resolution_hash: BytesN<32>, // Optional hash of resolution details
    pub resolution: u32, // Ruling: 1 = Upheld (Payer right), 2 = Rejected (Freelancer right)
    pub resolved_at: u64,
}

// ── Issue #34: LP priority queue events ───────────────────────────────────────

/// Emitted when an LP registers their intent to fund via the priority queue.
#[contractevent(topics = ["fund_requested"])]
#[derive(Clone, Debug, PartialEq)]
pub struct FundRequested {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub lp: Address,
    /// LP's reputation score at the time of registration.
    pub score: u32,
}

/// Emitted when the priority queue is resolved and a winning LP is selected.
#[contractevent(topics = ["fund_queue_resolved"])]
#[derive(Clone, Debug, PartialEq)]
pub struct FundQueueResolved {
    #[topic]
    pub invoice_id: u64,
    #[topic]
    pub approved_lp: Address,
    /// Winning score that secured priority.
    pub score: u32,
}
