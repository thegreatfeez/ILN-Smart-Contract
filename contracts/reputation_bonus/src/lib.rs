#![no_std]

pub mod config;
pub mod events;
pub mod invoice;
pub mod rate_logic;
pub mod reputation;
pub mod errors;

use crate::config::{get_config, set_admin, set_config, update_config, Config};
use crate::invoice::{submit_invoice, mark_paid, handle_default, Invoice};
use crate::reputation::{read_reputation, ReputationScore};
use crate::errors::ContractError;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
pub struct ReputationBonusContract;

#[contractimpl]
impl ReputationBonusContract {
    pub fn init(env: Env, admin: Address) {
        set_admin(&env, &admin);
    }

    pub fn set_config(env: Env, config: Config) -> Result<(), ContractError> {
        set_config(&env, &config).map_err(|_| ContractError::ConfigErrorUnauthorized)
    }

    pub fn get_config(env: Env) -> Result<Config, ContractError> {
        get_config(&env).map_err(|_| ContractError::ConfigErrorUnauthorized)
    }

    pub fn update_config(
        env: Env,
        caller: Address,
        high_rep_threshold: u32,
        bonus_bps: u32,
        min_discount_rate_bps: u32,
    ) -> Result<(), ContractError> {
        update_config(&env, &caller, high_rep_threshold, bonus_bps, min_discount_rate_bps)
            .map_err(|_| ContractError::ConfigErrorUnauthorized)
    }

    pub fn get_reputation(env: Env, address: Address) -> ReputationScore {
        read_reputation(&env, &address)
    }

    pub fn submit_invoice(
        env: Env,
        freelancer: Address,
        payer: Address,
        amount: i128,
        due_date: u64,
        base_discount_rate_bps: u32,
    ) -> Result<Invoice, ContractError> {
        submit_invoice(&env, &freelancer, &payer, amount, due_date, base_discount_rate_bps)
    }

    pub fn mark_paid(env: Env, invoice_id: u64) -> Result<(), ContractError> {
        mark_paid(&env, invoice_id)
    }

    pub fn handle_default(env: Env, invoice_id: u64) -> Result<(), ContractError> {
        handle_default(&env, invoice_id)
    }
}
