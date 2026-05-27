// SPDX-License-Identifier: MIT

//! Risk parameter management for credit lines.
//!
//! Provides admin-controlled functions to update borrower credit limits,
//! interest rates, and risk scores, with optional rate-change guardrails.

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::events::{publish_risk_parameters_updated, RiskParametersUpdatedEvent};
use crate::storage::rate_cfg_key;
use crate::types::{CreditLineData, RateChangeConfig};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score (0–100 scale).
pub const MAX_RISK_SCORE: u32 = 100;

/// Compute an interest rate in basis points from a normalised risk score.
///
/// Maps a borrower's risk score linearly onto the range
/// `[min_rate_bps, max_rate_bps]`. A score of `0` maps to `min_rate_bps`
/// (lowest risk, lowest rate) and a score of `100` maps to `max_rate_bps`
/// (highest risk, highest rate).
///
/// Formula:
/// ```text
/// rate = min_rate_bps + (max_rate_bps - min_rate_bps) * score / 100
/// ```
///
/// # Rounding
/// Truncates toward zero. For example, a spread of `999` bps over a score of
/// `1` yields `9` bps (`9.99` truncated), not `10`.
///
/// # Parameters
/// - `score`:        Borrower risk score in the range `0 ..= 100`.
///                   Values outside this range are accepted but produce
///                   extrapolated results; callers should validate first.
/// - `min_rate_bps`: Rate assigned to a score of `0` (best credit).
/// - `max_rate_bps`: Rate assigned to a score of `100` (worst credit).
///
/// # Returns
/// Interest rate in basis points for the given score, clamped implicitly by
/// the linear interpolation between `min_rate_bps` and `max_rate_bps`.
///
/// # Panics
/// - If `max_rate_bps < min_rate_bps` (invalid range).
///
/// # Example
/// ```
/// // Score 50 between 200 bps and 800 bps → midpoint 500 bps
/// assert_eq!(compute_rate_from_score(50, 200, 800), 500);
///
/// // Score 0 → min rate
/// assert_eq!(compute_rate_from_score(0, 200, 800), 200);
///
/// // Score 100 → max rate
/// assert_eq!(compute_rate_from_score(100, 200, 800), 800);
/// ```
pub fn compute_rate_from_score(score: u32, min_rate_bps: u32, max_rate_bps: u32) -> u32 {
    assert!(
        max_rate_bps >= min_rate_bps,
        "compute_rate_from_score: max_rate_bps must be >= min_rate_bps"
    );
    let spread = max_rate_bps - min_rate_bps;
    min_rate_bps + spread * score / 100
}

/// Update risk parameters for an existing credit line (admin only).
///
/// Loads the borrower's [`CreditLineData`], validates all inputs, applies
/// optional rate-change guardrails from [`RateChangeConfig`], then persists
/// the updated record and emits a [`RiskParametersUpdatedEvent`].
///
/// # Parameters
/// - `env`:              The Soroban environment.
/// - `borrower`:         Address of the borrower whose credit line to update.
/// - `credit_limit`:     New maximum borrowable amount. Must be `>= 0` and
///                       `>= credit_line.utilized_amount`.
/// - `interest_rate_bps`: New annual interest rate in basis points
///                       (`0 ..= 10_000`).
/// - `risk_score`:       New risk score (`0 ..= 100`).
///
/// # Panics
/// - If the caller is not the contract admin.
/// - If no credit line exists for `borrower`.
/// - If `credit_limit < 0`.
/// - If `credit_limit < credit_line.utilized_amount` (would strand debt above limit).
/// - If `interest_rate_bps > 10_000` (exceeds 100%).
/// - If `risk_score > 100`.
/// - If a [`RateChangeConfig`] is active and the absolute rate delta
///   `|new_rate - old_rate|` exceeds `max_rate_change_bps`.
/// - If a [`RateChangeConfig`] is active with `rate_change_min_interval > 0`,
///   a prior rate change exists, and the elapsed time since the last change
///   is less than `rate_change_min_interval`.
///
/// # Rate-change guardrails
/// When [`set_rate_change_limits`] has been called, every rate change is
/// subject to two additional checks:
///
/// 1. **Delta cap** — `|new_rate - old_rate| <= max_rate_change_bps`.
/// 2. **Interval floor** — seconds since `last_rate_update_ts` must be
///    `>= rate_change_min_interval` (skipped when `rate_change_min_interval`
///    is `0` or when no prior rate change has been recorded).
///
/// If the new rate equals the old rate, neither check is evaluated.
///
/// # Events
/// Emits [`RiskParametersUpdatedEvent`] on success.
pub fn update_risk_parameters(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    require_admin_auth(&env);

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    if credit_limit < 0 {
        panic!("credit_limit must be non-negative");
    }
    if credit_limit < credit_line.utilized_amount {
        panic!("credit_limit cannot be less than utilized amount");
    }
    if interest_rate_bps > MAX_INTEREST_RATE_BPS {
        panic!("interest_rate_bps exceeds maximum");
    }
    if risk_score > MAX_RISK_SCORE {
        panic!("risk_score exceeds maximum");
    }

    if interest_rate_bps != credit_line.interest_rate_bps {
        if let Some(cfg) = env
            .storage()
            .instance()
            .get::<_, RateChangeConfig>(&rate_cfg_key(&env))
        {
            let old_rate = credit_line.interest_rate_bps;
            let delta = interest_rate_bps.abs_diff(old_rate);

            if delta > cfg.max_rate_change_bps {
                panic!("rate change exceeds maximum allowed delta");
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts != 0 {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    panic!("rate change too soon: minimum interval not elapsed");
                }
            }
        }

        credit_line.last_rate_update_ts = env.ledger().timestamp();
    }

    credit_line.credit_limit = credit_limit;
    credit_line.interest_rate_bps = interest_rate_bps;
    credit_line.risk_score = risk_score;

    env.storage().persistent().set(&borrower, &credit_line);

    publish_risk_parameters_updated(
        &env,
        RiskParametersUpdatedEvent {
            borrower: borrower.clone(),
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

/// Configure rate-change guardrails (admin only).
///
/// Stores a [`RateChangeConfig`] that constrains future calls to
/// [`update_risk_parameters`] whenever the interest rate is being changed.
///
/// # Parameters
/// - `env`:                    The Soroban environment.
/// - `max_rate_change_bps`:    Maximum absolute change in `interest_rate_bps`
///                             allowed per [`update_risk_parameters`] call.
///                             Pass `u32::MAX` to effectively disable the cap.
/// - `rate_change_min_interval`: Minimum seconds that must elapse between
///                             consecutive rate changes. Pass `0` to disable
///                             the interval check.
///
/// # Panics
/// - If the caller is not the contract admin.
///
/// # Note
/// Calling this function again overwrites the previous configuration
/// atomically; there is no partial-update risk.
pub fn set_rate_change_limits(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
    require_admin_auth(&env);
    let cfg = RateChangeConfig {
        max_rate_change_bps,
        rate_change_min_interval,
    };
    env.storage().instance().set(&rate_cfg_key(&env), &cfg);
}

/// Return the current rate-change guardrail configuration, if any.
///
/// # Parameters
/// - `env`: The Soroban environment.
///
/// # Returns
/// `Some(RateChangeConfig)` if guardrails have been configured via
/// [`set_rate_change_limits`], or `None` if no configuration exists (meaning
/// rate changes are unconstrained).
pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
    env.storage().instance().get(&rate_cfg_key(&env))
}
