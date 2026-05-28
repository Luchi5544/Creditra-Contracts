// SPDX-License-Identifier: MIT

//! Pure integer arithmetic helpers used across the credit contract.

#![warn(missing_docs)]

/// Scaling factor used for fixed-point helpers.
pub const SCALE: u128 = 1_000_000_000_000_000_000_u128;

/// Number of basis points in 100%.
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Seconds in a 365-day year.
pub const SECONDS_PER_YEAR: u128 = 31_536_000;

const BPS_YEAR_DENOMINATOR: u128 = BPS_DENOMINATOR * SECONDS_PER_YEAR;

/// Rounding mode for integer division helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Truncate the remainder.
    Floor,
    /// Round up when a non-zero remainder exists.
    Ceil,
}

/// Multiply `value` by `numerator` and divide by `denominator`.
///
/// The result is rounded according to `rounding`.
pub fn mul_div(value: u128, numerator: u128, denominator: u128, rounding: Rounding) -> u128 {
    assert!(denominator != 0, "math_utils: division by zero");

    let product = value
        .checked_mul(numerator)
        .expect("math_utils: multiplication overflow");
    let quotient = product / denominator;

    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if product % denominator == 0 {
                quotient
            } else {
                quotient.checked_add(1).expect("math_utils: ceil overflow")
            }
        }
    }
}

/// Multiply `amount` by `SCALE`.
pub fn scale_up(amount: u128) -> u128 {
    amount
        .checked_mul(SCALE)
        .expect("math_utils: scale_up overflow")
}

/// Divide `amount` by `SCALE` using the requested rounding mode.
pub fn scale_down(amount: u128, rounding: Rounding) -> u128 {
    let quotient = amount / SCALE;

    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if amount % SCALE == 0 {
                quotient
            } else {
                quotient
                    .checked_add(1)
                    .expect("math_utils: scale_down ceil overflow")
            }
        }
    }
}

/// Apply a basis-point rate to an amount.
pub fn apply_bps(amount: u128, rate_bps: u32, rounding: Rounding) -> u128 {
    mul_div(amount, rate_bps as u128, BPS_DENOMINATOR, rounding)
}

/// Compute prorated interest for an elapsed time interval.
pub fn prorate_interest(
    principal: u128,
    rate_bps: u32,
    elapsed_secs: u64,
    rounding: Rounding,
) -> u128 {
    if principal == 0 || rate_bps == 0 || elapsed_secs == 0 {
        return 0;
    }

    let step1 = principal
        .checked_mul(rate_bps as u128)
        .expect("math_utils: prorate step1 overflow");
    let step2 = step1
        .checked_mul(elapsed_secs as u128)
        .expect("math_utils: prorate step2 overflow");

    let quotient = step2 / BPS_YEAR_DENOMINATOR;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if step2 % BPS_YEAR_DENOMINATOR == 0 {
                quotient
            } else {
                quotient
                    .checked_add(1)
                    .expect("math_utils: prorate ceil overflow")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_floor_and_ceil_behave_as_expected() {
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Floor), 300);
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);
    }

    #[test]
    fn apply_bps_matches_basic_basis_point_math() {
        assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
        assert_eq!(apply_bps(1, 1, Rounding::Floor), 0);
        assert_eq!(apply_bps(1, 1, Rounding::Ceil), 1);
    }

    #[test]
    fn prorate_interest_handles_zero_inputs() {
        assert_eq!(prorate_interest(0, 300, 86_400, Rounding::Floor), 0);
        assert_eq!(prorate_interest(10_000, 0, 86_400, Rounding::Floor), 0);
        assert_eq!(prorate_interest(10_000, 300, 0, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_matches_one_year_example() {
        assert_eq!(
            prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Floor),
            300
        );
    }

    #[test]
    fn scale_helpers_round_trip_on_exact_values() {
        let scaled = scale_up(42);
        assert_eq!(scale_down(scaled, Rounding::Floor), 42);
        assert_eq!(scale_down(scaled, Rounding::Ceil), 42);
    }
}
