// SPDX-License-Identifier: MIT

//! Interest accrual logic for credit lines.

#![warn(missing_docs)]

use crate::events::{publish_interest_accrued_event, InterestAccruedEvent};
use crate::types::{
    ContractError, CreditLineData, CreditStatus, GracePeriodConfig, GraceWaiverMode,
};
use soroban_sdk::Env;

/// Seconds in a 365-day year.
pub(crate) const SECONDS_PER_YEAR: u64 = 31_536_000;

/// Compute simple interest: `utilized * rate_bps * seconds / (10_000 * SECONDS_PER_YEAR)`.
fn compute_interest(utilized: i128, rate_bps: i128, seconds: i128) -> Result<i128, ContractError> {
    let denominator: i128 = 10_000 * (SECONDS_PER_YEAR as i128);
    let intermediate = utilized
        .checked_mul(rate_bps)
        .and_then(|value| value.checked_mul(seconds));

    match intermediate {
        Some(value) => Ok(value / denominator),
        None => Err(ContractError::Overflow),
    }
}

/// Apply interest accrual to a credit line and return the updated line.
///
/// When the line is suspended and a grace-period policy exists, the effective
/// rate may be reduced or waived for the in-window portion of elapsed time.
pub fn apply_accrual(env: &Env, mut line: CreditLineData) -> CreditLineData {
    let now = env.ledger().timestamp();

    if now <= line.last_accrual_ts {
        return line;
    }

    if line.utilized_amount == 0 {
        line.last_accrual_ts = now;
        return line;
    }

    let utilized = line.utilized_amount;
    let full_rate = line.interest_rate_bps as i128;
    let accrual_start = line.last_accrual_ts;

    let accrued = if line.status == CreditStatus::Suspended {
        let grace_cfg: Option<GracePeriodConfig> = env
            .storage()
            .instance()
            .get(&crate::storage::grace_period_key(env));

        match grace_cfg {
            Some(cfg) if cfg.grace_period_seconds > 0 => {
                let grace_end = line.suspension_ts.saturating_add(cfg.grace_period_seconds);

                if now <= grace_end {
                    let seconds = (now - accrual_start) as i128;
                    match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0,
                        GraceWaiverMode::ReducedRate => {
                            compute_interest(utilized, cfg.reduced_rate_bps as i128, seconds)
                                .unwrap_or_else(|err| env.panic_with_error(err))
                        }
                    }
                } else if accrual_start >= grace_end {
                    let seconds = (now - accrual_start) as i128;
                    compute_interest(utilized, full_rate, seconds)
                        .unwrap_or_else(|err| env.panic_with_error(err))
                } else {
                    let in_window_secs = (grace_end - accrual_start) as i128;
                    let post_window_secs = (now - grace_end) as i128;

                    let in_window_interest = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0,
                        GraceWaiverMode::ReducedRate => {
                            compute_interest(utilized, cfg.reduced_rate_bps as i128, in_window_secs)
                                .unwrap_or_else(|err| env.panic_with_error(err))
                        }
                    };

                    let post_window_interest =
                        compute_interest(utilized, full_rate, post_window_secs)
                            .unwrap_or_else(|err| env.panic_with_error(err));

                    in_window_interest
                        .checked_add(post_window_interest)
                        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
                }
            }
            _ => {
                let seconds = (now - accrual_start) as i128;
                compute_interest(utilized, full_rate, seconds)
                    .unwrap_or_else(|err| env.panic_with_error(err))
            }
        }
    } else {
        let seconds = (now - accrual_start) as i128;
        compute_interest(utilized, full_rate, seconds)
            .unwrap_or_else(|err| env.panic_with_error(err))
    };

    if accrued > 0 {
        line.utilized_amount = line
            .utilized_amount
            .checked_add(accrued)
            .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
        line.accrued_interest = line
            .accrued_interest
            .checked_add(accrued)
            .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

        publish_interest_accrued_event(
            env,
            InterestAccruedEvent {
                borrower: line.borrower.clone(),
                accrued_amount: accrued,
                new_utilized_amount: line.utilized_amount,
            },
        );
    }

    line.last_accrual_ts = now;
    line
}
