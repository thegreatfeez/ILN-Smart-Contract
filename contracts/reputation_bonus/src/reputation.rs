use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationScore {
    pub invoices_submitted: u32,
    pub invoices_paid: u32,
    pub invoices_defaulted: u32,
    pub score: u32,
}

#[contracttype]
pub enum ReputationKey {
    Reputation(Address),
}

const REPUTATION_UPDATED: Symbol = symbol_short!("REP_UPD");

impl ReputationScore {
    pub fn new() -> Self {
        ReputationScore {
            invoices_submitted: 0,
            invoices_paid: 0,
            invoices_defaulted: 0,
            score: 0,
        }
    }

    pub fn recalculate_score(&mut self) {
        let divisor = self.invoices_submitted.max(1);
        self.score = (self.invoices_paid as u64)
            .checked_mul(100)
            .and_then(|v| v.checked_div(divisor as u64))
            .map(|v| v as u32)
            .unwrap_or(0)
            .min(100);
    }
}

pub fn read_reputation(env: &Env, address: &Address) -> ReputationScore {
    env.storage()
        .persistent()
        .get(&ReputationKey::Reputation(address.clone()))
        .unwrap_or_else(|| ReputationScore::new())
}

pub fn write_reputation(env: &Env, address: &Address, mut reputation: ReputationScore) {
    reputation.recalculate_score();
    env.storage()
        .persistent()
        .set(&ReputationKey::Reputation(address.clone()), &reputation);

    env.events().publish(
        (REPUTATION_UPDATED, address.clone()),
        (
            reputation.invoices_submitted,
            reputation.invoices_paid,
            reputation.invoices_defaulted,
            reputation.score,
        ),
    );
}
