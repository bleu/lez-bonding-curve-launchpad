//! Pricing arithmetic for the bonding curve, as pure functions over integers.
//!
//! No accounts, no state, no chain. The job of this crate is to hide the rounding
//! discipline and the overflow bounds behind named functions, so that the rounding
//! direction at every call site is visible at the call rather than inferred from
//! context.
//!
//! These are honest quotes over the current reserves. Rounding drift accumulates
//! as `Vt * Vc` moves above `k`, and a quote treats that surplus as spendable:
//! after ordinary trades, a zero-amount trade can quote a positive payout. Callers
//! must reject zero-amount trades; the state machine in `crates/sale` owns that.
//!
//! The u128 bound argument lives in `docs/adr/0004`.

/// Why a pricing call could not produce a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    /// An intermediate operation left u128. The ADR 0004 bounds make the state
    /// safe on its own, but trade amounts are unbounded input, so this arm is
    /// a real guard, not a formality.
    Overflow,
    /// The trade asks for more than the virtual reserves back.
    ExceedsVirtualReserves,
}

// Hand-written: the dependency rules keep `thiserror` out of this crate.
impl std::fmt::Display for MathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overflow => write!(f, "an intermediate operation left u128"),
            Self::ExceedsVirtualReserves => {
                write!(f, "the trade asks for more than the virtual reserves back")
            }
        }
    }
}

impl std::error::Error for MathError {}

/// Tokens dispensed for `c_in` collateral: `Vt - k / (Vc + C_in)`, rounded down.
pub fn buy_tokens_out_floor(vt: u128, vc: u128, k: u128, c_in: u128) -> Result<u128, MathError> {
    let collateral = vc.checked_add(c_in).ok_or(MathError::Overflow)?;
    let quotient = div_ceil(k, collateral)?;
    vt.checked_sub(quotient)
        .ok_or(MathError::ExceedsVirtualReserves)
}

/// Collateral required for exactly `q` tokens: `k / (Vt - Q) - Vc`, rounded up.
pub fn buy_collateral_in_ceil(vt: u128, vc: u128, k: u128, q: u128) -> Result<u128, MathError> {
    let remaining = vt.checked_sub(q).ok_or(MathError::ExceedsVirtualReserves)?;
    let target = div_ceil(k, remaining)?;
    target
        .checked_sub(vc)
        .ok_or(MathError::ExceedsVirtualReserves)
}

/// Collateral paid for `tokens_in` sold back: `Vc - k / (Vt + tokens_in)`, rounded down.
pub fn sell_collateral_out_floor(
    vt: u128,
    vc: u128,
    k: u128,
    tokens_in: u128,
) -> Result<u128, MathError> {
    let tokens = vt.checked_add(tokens_in).ok_or(MathError::Overflow)?;
    let quotient = div_ceil(k, tokens)?;
    vc.checked_sub(quotient)
        .ok_or(MathError::ExceedsVirtualReserves)
}

/// `n / d` rounded up. Flooring `V - k/x` means ceiling `k/x`.
///
/// `d` is always a reserve plus or minus a trade amount, so `d == 0` means the
/// reserves cannot back the trade, not an arithmetic accident.
fn div_ceil(n: u128, d: u128) -> Result<u128, MathError> {
    let floored = n.checked_div(d).ok_or(MathError::ExceedsVirtualReserves)?;
    let remainder = n.checked_rem(d).ok_or(MathError::ExceedsVirtualReserves)?;
    if remainder == 0 {
        Ok(floored)
    } else {
        floored.checked_add(1).ok_or(MathError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Creation-shaped state throughout: k = Vt * Vc.

    #[test]
    fn buy_dispenses_the_formula_amount_when_the_division_is_exact() {
        // Vt = 1000, Vc = 100, k = 100_000. C_in = 25: 1000 - 100_000 / 125 = 200.
        assert_eq!(buy_tokens_out_floor(1000, 100, 100_000, 25), Ok(200));
    }

    #[test]
    fn buy_rounds_the_dispensed_tokens_down() {
        // C_in = 24: 1000 - 100_000 / 124 = 193.55, so the participant gets 193.
        assert_eq!(buy_tokens_out_floor(1000, 100, 100_000, 24), Ok(193));
    }

    #[test]
    fn buy_refuses_a_state_whose_curve_point_is_above_the_virtual_token_reserve() {
        // k / (Vc + C_in) = 1000 exceeds Vt = 10; the subtraction has no answer.
        assert_eq!(
            buy_tokens_out_floor(10, 100, 100_000, 0),
            Err(MathError::ExceedsVirtualReserves)
        );
    }

    #[test]
    fn collateral_in_matches_the_inverse_formula_when_the_division_is_exact() {
        // Q = 200: 100_000 / (1000 - 200) - 100 = 25. The inverse of the exact buy above.
        assert_eq!(buy_collateral_in_ceil(1000, 100, 100_000, 200), Ok(25));
    }

    #[test]
    fn collateral_in_rounds_up_against_the_participant() {
        // Q = 100: 100_000 / 900 - 100 = 11.11, so the participant pays 12.
        assert_eq!(buy_collateral_in_ceil(1000, 100, 100_000, 100), Ok(12));
    }

    #[test]
    fn collateral_in_refuses_a_quantity_at_or_above_the_virtual_token_reserve() {
        // The curve dispenses strictly less than Vt, however much collateral comes in.
        assert_eq!(
            buy_collateral_in_ceil(1000, 100, 100_000, 1000),
            Err(MathError::ExceedsVirtualReserves)
        );
        assert_eq!(
            buy_collateral_in_ceil(1000, 100, 100_000, 1001),
            Err(MathError::ExceedsVirtualReserves)
        );
    }

    #[test]
    fn sell_pays_the_formula_amount_when_the_division_is_exact() {
        // Tokens in = 250: 100 - 100_000 / 1250 = 20.
        assert_eq!(sell_collateral_out_floor(1000, 100, 100_000, 250), Ok(20));
    }

    #[test]
    fn sell_rounds_the_payout_down() {
        // Tokens in = 240: 100 - 100_000 / 1240 = 19.35, so the participant gets 19.
        assert_eq!(sell_collateral_out_floor(1000, 100, 100_000, 240), Ok(19));
    }

    #[test]
    fn sell_refuses_a_state_whose_curve_point_is_above_the_virtual_collateral_reserve() {
        // k / (Vt + tokens_in) = 80 exceeds Vc = 50; the payout is not backed.
        assert_eq!(
            sell_collateral_out_floor(1000, 50, 100_000, 250),
            Err(MathError::ExceedsVirtualReserves)
        );
    }

    #[test]
    fn buy_reports_overflow_instead_of_wrapping() {
        assert_eq!(
            buy_tokens_out_floor(1000, u128::MAX, 100_000, 1),
            Err(MathError::Overflow)
        );
    }

    #[test]
    fn sell_reports_overflow_instead_of_wrapping() {
        assert_eq!(
            sell_collateral_out_floor(u128::MAX, 100, 100_000, 1),
            Err(MathError::Overflow)
        );
    }
}
