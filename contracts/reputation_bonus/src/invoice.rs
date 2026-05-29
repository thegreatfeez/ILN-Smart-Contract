use crate::config::get_config;
use crate::rate_logic::calculate_effective_rate;
use crate::reputation::{read_reputation, write_reputation};
use crate::errors::ContractError;
use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum InvoiceStatus {
    Pending,
    Funded,
    Paid,
    Defaulted,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Invoice {
    pub id: u64,
    pub freelancer: Address,
    pub payer: Address,
    pub amount: i128,
    pub due_date: u64,
    pub base_discount_rate_bps: u32,
    pub effective_discount_rate_bps: u32,
    pub status: InvoiceStatus,
}

#[contracttype]
pub enum InvoiceKey {
    Invoice(u64),
    InvoiceCount,
}

pub fn submit_invoice(
    env: &Env,
    freelancer: &Address,
    payer: &Address,
    amount: i128,
    due_date: u64,
    base_discount_rate_bps: u32,
) -> Result<Invoice, ContractError> {
    freelancer.require_auth();

    let config = get_config(env).map_err(|_| ContractError::ConfigErrorUnauthorized)?;
    let rep_score = read_reputation(env, freelancer);

    let effective_rate = calculate_effective_rate(
        base_discount_rate_bps,
        rep_score.score,
        config.high_rep_threshold,
        config.bonus_bps,
        config.min_discount_rate_bps,
    )
    .map_err(|_| ContractError::RateErrorArithmeticOverflow)?;

    let count: u64 = env
        .storage()
        .instance()
        .get(&InvoiceKey::InvoiceCount)
        .unwrap_or(0);
    let next_id = count.checked_add(1).ok_or(ContractError::ArithmeticError)?;
    env.storage().instance().set(&InvoiceKey::InvoiceCount, &next_id);

    let invoice = Invoice {
        id: next_id,
        freelancer: freelancer.clone(),
        payer: payer.clone(),
        amount,
        due_date,
        base_discount_rate_bps,
        effective_discount_rate_bps: effective_rate,
        status: InvoiceStatus::Pending,
    };

    env.storage()
        .persistent()
        .set(&InvoiceKey::Invoice(next_id), &invoice);

    // Update reputation: increment submitted for both parties
    let mut free_rep = read_reputation(env, freelancer);
    free_rep.invoices_submitted = free_rep.invoices_submitted.saturating_add(1);
    write_reputation(env, freelancer, free_rep);

    let mut payer_rep = read_reputation(env, payer);
    payer_rep.invoices_submitted = payer_rep.invoices_submitted.saturating_add(1);
    write_reputation(env, payer, payer_rep);

    Ok(invoice)
}

pub fn mark_paid(env: &Env, invoice_id: u64) -> Result<(), ContractError> {
    let mut invoice: Invoice = env.storage()
        .persistent()
        .get(&InvoiceKey::Invoice(invoice_id))
        .ok_or(ContractError::InvoiceNotFound)?;

    if invoice.status != InvoiceStatus::Pending && invoice.status != InvoiceStatus::Funded {
        return Err(ContractError::IllegalState);
    }

    invoice.payer.require_auth();
    invoice.status = InvoiceStatus::Paid;

    env.storage().persistent().set(&InvoiceKey::Invoice(invoice_id), &invoice);

    // Update reputation: increment paid for both parties
    let mut payer_rep = read_reputation(env, &invoice.payer);
    payer_rep.invoices_paid = payer_rep.invoices_paid.saturating_add(1);
    write_reputation(env, &invoice.payer, payer_rep);

    let mut free_rep = read_reputation(env, &invoice.freelancer);
    free_rep.invoices_paid = free_rep.invoices_paid.saturating_add(1);
    write_reputation(env, &invoice.freelancer, free_rep);

    Ok(())
}

pub fn handle_default(env: &Env, invoice_id: u64) -> Result<(), ContractError> {
    let mut invoice: Invoice = env.storage()
        .persistent()
        .get(&InvoiceKey::Invoice(invoice_id))
        .ok_or(ContractError::InvoiceNotFound)?;

    if invoice.status != InvoiceStatus::Pending && invoice.status != InvoiceStatus::Funded {
        return Err(ContractError::IllegalState);
    }

    invoice.status = InvoiceStatus::Defaulted;
    env.storage().persistent().set(&InvoiceKey::Invoice(invoice_id), &invoice);

    // Update reputation: increment defaulted for both parties
    let mut payer_rep = read_reputation(env, &invoice.payer);
    payer_rep.invoices_defaulted = payer_rep.invoices_defaulted.saturating_add(1);
    write_reputation(env, &invoice.payer, payer_rep);

    let mut free_rep = read_reputation(env, &invoice.freelancer);
    free_rep.invoices_defaulted = free_rep.invoices_defaulted.saturating_add(1);
    write_reputation(env, &invoice.freelancer, free_rep);

    Ok(())
}
