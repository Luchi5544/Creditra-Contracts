// SPDX-License-Identifier: MIT

//! Pure integer arithmetic helpers used across the credit contract.
//!
//! All functions in this module operate on fixed-point integers and never
//! allocate. Rounding behaviour is documented per function; the default is
//! **truncation toward zero** (Rust's native integer division) unless stated
//! otherwise.

#![warn(missing_docs)]

/// Multiply `value` by `numerator` then divide by `denominator`, using an
/// intermediate `i128` accumulator to avoid overflow on typical inputs.
///
/// # Rounding
/// Truncates toward zero (floor for positive results). No rounding-up variant
/// is provided; callers that need ceiling arithmetic should add
/// `denominator - 1` to `value * numerator` before calling.
///
/// # Parameters
/// - `value`:       The base amount to scale.
/// - `numerator`:   Scaling numerator (e.g. an interest rate).
/// - `denominator`: Scaling denominator (e.g. 10_000 for basis-point math).
///
/// # Returns
/// `(value * numerator) / denominator`, truncated toward zero.
///
/// # Panics
/// - If `denominator` is zero (division by zero).
/// - If the intermediate product `value * numerator` overflows `i128`
///   (unlikely in practice; `i128` supports values up to ~1.7 × 10³⁸).
///
/// # Example
/// ```
/// // 1_000 * 300 / 10_000 = 30  (3% of 1_000)
/// assert_eq!(mul_div(1_000, 300, 10_000), 30);
/// ```
pub fn mul_div(value: i128, numerator: i128, denominator: i128) -> i128 {
    assert!(denominator != 0, "mul_div: denominator must not be zero");
    value
        .checked_mul(numerator)
        .expect("mul_div: intermediate product overflowed i128")
        / denominator
}

/// Apply a basis-point rate to an amount.
///
/// Basis points (bps) express rates as integer hundredths of a percent:
/// 1 bps = 0.01%, 100 bps = 1%, 10_000 bps = 100%.
///
/// This is a thin wrapper around [`mul_div`] with `denominator = 10_000`.
///
/// # Rounding
/// Truncates toward zero. For example, `apply_bps(1, 1)` returns `0`
/// because `1 * 1 / 10_000 = 0` after truncation.
///
/// # Parameters
/// - `amount`: The principal amount to apply the rate to.
/// - `rate_bps`: The rate in basis points (0 ..= 10_000 for 0%–100%;
///   values above 10_000 are accepted but represent rates over 100%).
///
/// # Returns
/// `amount * rate_bps / 10_000`, truncated toward zero.
///
/// # Panics
/// Panics only if the intermediate product `amount * rate_bps` overflows
/// `i128`, which requires both operands to be astronomically large.
///
/// # Examples
/// ```
/// // 3% of 1_000 = 30
/// assert_eq!(apply_bps(1_000, 300), 30);
///
/// // 0.5% of 200 = 1  (1.0 truncated to 1)
/// assert_eq!(apply_bps(200, 50), 1);
///
/// // 0.01% of 50 = 0  (0.005 truncated to 0)
/// assert_eq!(apply_bps(50, 1), 0);
///
/// // 100% of 500 = 500
/// assert_eq!(apply_bps(500, 10_000), 500);
/// ```
pub fn apply_bps(amount: i128, rate_bps: u32) -> i128 {
    mul_div(amount, rate_bps as i128, 10_000)
}

/// Pro-rate an annual interest charge to a sub-year elapsed period.
///
/// Converts an annual basis-point rate into the interest due for `elapsed`
/// seconds, assuming a 365-day (31_536_000-second) year.
///
/// Formula:
/// ```text
/// interest = principal * rate_bps * elapsed
///            ────────────────────────────────
///                  10_000 * 31_536_000
/// ```
///
/// Both multiplications are performed in `i128` to preserve precision before
/// the final division; the combined denominator is `315_360_000_000`.
///
/// # Rounding
/// Truncates toward zero. Partial-second or sub-unit amounts are lost.
/// For a principal of 1_000_000 at 500 bps (5%) over 1 hour (3_600 s):
/// ```text
/// 1_000_000 * 500 * 3_600 / 315_360_000_000
///   = 1_800_000_000_000 / 315_360_000_000
///   ≈ 5  (5 units of interest, truncated)
/// ```
///
/// # Parameters
/// - `principal`:   Outstanding balance to accrue interest on.
/// - `rate_bps`:    Annual interest rate in basis points (e.g. 500 = 5%).
/// - `elapsed_secs`: Seconds elapsed since last accrual. Passing `0` always
///   returns `0`.
///
/// # Returns
/// The pro-rated interest amount for the elapsed period, truncated toward zero.
///
/// # Panics
/// - If any intermediate multiplication overflows `i128`. In practice this
///   requires `principal * rate_bps` to exceed ~1.7 × 10³⁸, which is far
///   beyond realistic credit limits.
///
/// # Examples
/// ```
/// // 5% annual on 1_000_000 for 1 day (86_400 s)
/// // = 1_000_000 * 500 * 86_400 / 315_360_000_000
/// // = 43_200_000_000_000 / 315_360_000_000 = 137 (truncated)
/// assert_eq!(prorate_interest(1_000_000, 500, 86_400), 137);
///
/// // Zero elapsed → always 0
/// assert_eq!(prorate_interest(1_000_000, 500, 0), 0);
///
/// // Zero principal → always 0
/// assert_eq!(prorate_interest(0, 500, 86_400), 0);
///
/// // 10% annual on 100_000 for 1 year (31_536_000 s) = 10_000 exactly
/// assert_eq!(prorate_interest(100_000, 1_000, 31_536_000), 10_000);
/// ```
pub fn prorate_interest(principal: i128, rate_bps: u32, elapsed_secs: u64) -> i128 {
    const SECONDS_PER_YEAR: i128 = 31_536_000;
    const BPS_DENOMINATOR: i128 = 10_000;

    if elapsed_secs == 0 || principal == 0 {
        return 0;
    }

    let numerator = principal
        .checked_mul(rate_bps as i128)
        .expect("prorate_interest: principal * rate_bps overflowed i128")
        .checked_mul(elapsed_secs as i128)
        .expect("prorate_interest: product with elapsed_secs overflowed i128");

    let denominator = BPS_DENOMINATOR
        .checked_mul(SECONDS_PER_YEAR)
        .expect("prorate_interest: denominator overflowed i128");

    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── mul_div ──────────────────────────────────────────────────────────────

    #[test]
    fn mul_div_basic() {
        assert_eq!(mul_div(1_000, 300, 10_000), 30);
    }

    #[test]
    fn mul_div_truncates_toward_zero() {
        // 7 * 1 / 3 = 2.33… → 2
        assert_eq!(mul_div(7, 1, 3), 2);
    }

    #[test]
    fn mul_div_identity_denominator() {
        assert_eq!(mul_div(42, 1, 1), 42);
    }

    #[test]
    #[should_panic(expected = "denominator must not be zero")]
    fn mul_div_zero_denominator_panics() {
        mul_div(1, 1, 0);
    }

    // ── apply_bps ────────────────────────────────────────────────────────────

    #[test]
    fn apply_bps_three_percent() {
        assert_eq!(apply_bps(1_000, 300), 30);
    }

    #[test]
    fn apply_bps_half_percent_truncates() {
        assert_eq!(apply_bps(200, 50), 1);
    }

    #[test]
    fn apply_bps_sub_unit_truncates_to_zero() {
        assert_eq!(apply_bps(50, 1), 0);
    }

    #[test]
    fn apply_bps_full_rate() {
        assert_eq!(apply_bps(500, 10_000), 500);
    }

    #[test]
    fn apply_bps_zero_rate() {
        assert_eq!(apply_bps(1_000_000, 0), 0);
    }

    // ── prorate_interest ─────────────────────────────────────────────────────

    #[test]
    fn prorate_interest_one_day() {
        // 5% annual on 1_000_000 for 1 day
        assert_eq!(prorate_interest(1_000_000, 500, 86_400), 137);
    }

    #[test]
    fn prorate_interest_zero_elapsed() {
        assert_eq!(prorate_interest(1_000_000, 500, 0), 0);
    }

    #[test]
    fn prorate_interest_zero_principal() {
        assert_eq!(prorate_interest(0, 500, 86_400), 0);
    }

    #[test]
    fn prorate_interest_full_year() {
        // 10% on 100_000 for exactly 1 year = 10_000
        assert_eq!(prorate_interest(100_000, 1_000, 31_536_000), 10_000);
    }

    #[test]
    fn prorate_interest_one_hour() {
        // 5% on 1_000_000 for 3_600 s ≈ 5
        assert_eq!(prorate_interest(1_000_000, 500, 3_600), 5);
    }
}
EOF</parameter>
<parameter name="description">Write math_utils.rs</parameter>
cat > contracts/credit/src/accrual.rs << 'EOF'
// SPDX-License-Identifier: MIT

//! Interest accrual logic for credit lines.
//!
//! This module computes and applies pro-rated interest to a [`CreditLineData`]
//! record. Interest is calculated using a 365-day year and capitalised into
//! `accrued_interest`; it does **not** automatically increase `utilized_amount`
//! — the caller decides when to capitalise.

#![warn(missing_docs)]

use crate::math_utils::prorate_interest;
use crate::types::CreditLineData;
use soroban_sdk::Env;

/// Compute and apply accrued interest to a credit line for the elapsed period.
///
/// Calculates the interest owed since `credit_line.last_accrual_ts` using
/// [`prorate_interest`], adds it to `credit_line.accrued_interest`, and
/// updates `credit_line.last_accrual_ts` to `now`.
///
/// # How interest is computed
/// ```text
/// elapsed  = now - last_accrual_ts          (seconds)
/// interest = principal * rate_bps * elapsed
///            ────────────────────────────────
///                  10_000 * 31_536_000
/// ```
/// where `principal` is `credit_line.utilized_amount` and `rate_bps` is
/// `credit_line.interest_rate_bps`.
///
/// # Rounding
/// Truncates toward zero via [`prorate_interest`]. Sub-unit interest amounts
/// (e.g. tiny principals or very short elapsed windows) accrue as `0` for
/// that period and are not carried forward.
///
/// # Parameters
/// - `env`:         The Soroban environment; used to read the current ledger
///                  timestamp via `env.ledger().timestamp()`.
/// - `credit_line`: Mutable reference to the credit line to update. Both
///                  `accrued_interest` and `last_accrual_ts` are modified
///                  in-place. The caller is responsible for persisting the
///                  updated record to storage.
///
/// # Returns
/// The amount of interest accrued in this call (may be `0` if `elapsed == 0`,
/// `utilized_amount == 0`, or the computed amount truncates to zero).
///
/// # Panics
/// - If `principal * rate_bps * elapsed` overflows `i128` (see
///   [`prorate_interest`] for bounds). Not expected under realistic credit
///   limits and rates.
/// - If adding newly accrued interest to `credit_line.accrued_interest`
///   overflows `i128` (would require astronomically large cumulative interest).
///
/// # Example
/// ```
/// // Credit line: 1_000_000 utilized at 500 bps (5% p.a.)
/// // last_accrual_ts = 0, now = 86_400 (1 day later)
/// // interest = 1_000_000 * 500 * 86_400 / 315_360_000_000 = 137
/// // After call: accrued_interest += 137, last_accrual_ts = 86_400
/// ```
pub fn apply_accrual(env: &Env, credit_line: &mut CreditLineData) -> i128 {
    let now = env.ledger().timestamp();
    let last = credit_line.last_accrual_ts;

    let elapsed = now.saturating_sub(last);

    let interest = prorate_interest(
        credit_line.utilized_amount,
        credit_line.interest_rate_bps,
        elapsed,
    );

    credit_line.accrued_interest = credit_line
        .accrued_interest
        .checked_add(interest)
        .expect("apply_accrual: accrued_interest overflowed i128");

    credit_line.last_accrual_ts = now;

    interest
}
