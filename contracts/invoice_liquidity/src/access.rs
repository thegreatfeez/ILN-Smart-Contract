use soroban_sdk::{Address, Env};
use crate::invoice::{load_invoice, invoice_exists, get_invoice_funders, StorageKey};
use crate::errors::ContractError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Role {
    Submitter,
    Payer,
    LP,
    Admin,
    Governance,
    Anyone,
}

pub fn require_admin(env: &Env) -> Result<(), ContractError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(ContractError::Unauthorized)?;
    admin.require_auth();
    Ok(())
}

pub fn require_submitter(env: &Env, caller: &Address) -> Result<(), ContractError> {
    caller.require_auth();
    Ok(())
}

pub fn require_submitter_by_id(env: &Env, caller: &Address, invoice_id: u64) -> Result<(), ContractError> {
    if !invoice_exists(env, invoice_id) {
        return Err(ContractError::InvoiceNotFound);
    }
    let invoice = load_invoice(env, invoice_id);
    caller.require_auth();
    if caller != &invoice.freelancer {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

pub fn require_payer_by_id(env: &Env, invoice_id: u64) -> Result<(), ContractError> {
    if !invoice_exists(env, invoice_id) {
        return Err(ContractError::InvoiceNotFound);
    }
    let invoice = load_invoice(env, invoice_id);
    invoice.payer.require_auth();
    Ok(())
}

pub fn require_lp(env: &Env, caller: &Address) -> Result<(), ContractError> {
    caller.require_auth();
    Ok(())
}

pub fn require_lp_by_id(env: &Env, caller: &Address, invoice_id: u64) -> Result<(), ContractError> {
    if !invoice_exists(env, invoice_id) {
        return Err(ContractError::InvoiceNotFound);
    }
    caller.require_auth();
    
    let funders = get_invoice_funders(env, invoice_id);
    let mut is_funder = false;
    for i in 0..funders.len() {
        if funders.get(i).unwrap().0 == *caller {
            is_funder = true;
            break;
        }
    }
    if !is_funder {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

pub fn require_governance(_env: &Env) -> Result<(), ContractError> {
    // Currently no governance implemented, always reject
    Err(ContractError::Unauthorized)
}
