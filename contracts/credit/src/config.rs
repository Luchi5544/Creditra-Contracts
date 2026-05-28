// SPDX-License-Identifier: MIT

//! Contract initialization helpers.

use crate::storage::{admin_key, set_schema_version, DataKey};
use crate::types::ContractError;
use soroban_sdk::{Address, Env};

/// Initialize the contract exactly once.
pub fn init(env: Env, admin: Address) {
    let key = admin_key(&env);
    if env.storage().instance().has(&key) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }

    env.storage().instance().set(&key, &admin);
    env.storage()
        .instance()
        .set(&DataKey::LiquiditySource, &env.current_contract_address());
    env.storage()
        .instance()
        .set(&DataKey::CreditLineCount, &0_u32);
    env.storage()
        .instance()
        .set(&DataKey::TotalUtilized, &0_i128);
    set_schema_version(&env, crate::SCHEMA_VERSION);
}
