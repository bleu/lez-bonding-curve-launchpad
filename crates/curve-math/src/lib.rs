//! Pricing arithmetic for the bounded constant-product pool.
//!
//! No accounts, no state, no chain. The job of this crate is to hide the rounding
//! discipline and the overflow bounds behind named functions, so that the rounding
//! direction at every call site is visible at the call rather than inferred from
//! context.
//!
//! These are honest quotes over the invariant product supplied by the caller. The
//! state machine in `crates/pool` supplies the current reserve product so rounding
//! surplus is not spendable, and rejects zero-amount trades before quoting.
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

/// Output for an exact input: `reserve_out - k / (reserve_in + amount_in)`, rounded down.
pub fn exact_input_amount_out_floor(
    reserve_in: u128,
    reserve_out: u128,
    k: u128,
    amount_in: u128,
) -> Result<u128, MathError> {
    let new_reserve_in = reserve_in
        .checked_add(amount_in)
        .ok_or(MathError::Overflow)?;
    let quotient = div_ceil(k, new_reserve_in)?;
    reserve_out
        .checked_sub(quotient)
        .ok_or(MathError::ExceedsVirtualReserves)
}

/// Input required for an exact output: `k / (reserve_out - amount_out) - reserve_in`, rounded up.
pub fn exact_output_amount_in_ceil(
    reserve_in: u128,
    reserve_out: u128,
    k: u128,
    amount_out: u128,
) -> Result<u128, MathError> {
    let remaining = reserve_out
        .checked_sub(amount_out)
        .ok_or(MathError::ExceedsVirtualReserves)?;
    let target = div_ceil(k, remaining)?;
    target
        .checked_sub(reserve_in)
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
    fn exact_input_returns_the_formula_amount_when_the_division_is_exact() {
        // Vt = 1000, Vc = 100, k = 100_000. C_in = 25: 1000 - 100_000 / 125 = 200.
        assert_eq!(
            exact_input_amount_out_floor(100, 1000, 100_000, 25),
            Ok(200)
        );
    }

    #[test]
    fn exact_input_rounds_output_down() {
        // C_in = 24: 1000 - 100_000 / 124 = 193.55, so the participant gets 193.
        assert_eq!(
            exact_input_amount_out_floor(100, 1000, 100_000, 24),
            Ok(193)
        );
    }

    #[test]
    fn exact_input_refuses_a_curve_point_above_the_virtual_output_reserve() {
        // k / (Vc + C_in) = 1000 exceeds Vt = 10; the subtraction has no answer.
        assert_eq!(
            exact_input_amount_out_floor(100, 10, 100_000, 0),
            Err(MathError::ExceedsVirtualReserves)
        );
    }

    #[test]
    fn exact_output_input_matches_the_inverse_formula_when_division_is_exact() {
        // Output = 200: 100_000 / (1000 - 200) - 100 = 25.
        assert_eq!(exact_output_amount_in_ceil(100, 1000, 100_000, 200), Ok(25));
    }

    #[test]
    fn exact_output_rounds_input_up() {
        // Q = 100: 100_000 / 900 - 100 = 11.11, so the participant pays 12.
        assert_eq!(exact_output_amount_in_ceil(100, 1000, 100_000, 100), Ok(12));
    }

    #[test]
    fn exact_output_refuses_output_at_or_above_the_virtual_output_reserve() {
        assert_eq!(
            exact_output_amount_in_ceil(100, 1000, 100_000, 1000),
            Err(MathError::ExceedsVirtualReserves)
        );
        assert_eq!(
            exact_output_amount_in_ceil(100, 1000, 100_000, 1001),
            Err(MathError::ExceedsVirtualReserves)
        );
    }

    #[test]
    fn reverse_exact_input_returns_the_formula_amount_when_division_is_exact() {
        // Tokens in = 250: 100 - 100_000 / 1250 = 20.
        assert_eq!(
            exact_input_amount_out_floor(1000, 100, 100_000, 250),
            Ok(20)
        );
    }

    #[test]
    fn reverse_exact_input_rounds_output_down() {
        // Tokens in = 240: 100 - 100_000 / 1240 = 19.35, so the participant gets 19.
        assert_eq!(
            exact_input_amount_out_floor(1000, 100, 100_000, 240),
            Ok(19)
        );
    }

    #[test]
    fn reverse_exact_input_refuses_a_curve_point_above_virtual_output() {
        // k / (Vt + tokens_in) = 80 exceeds Vc = 50; the payout is not backed.
        assert_eq!(
            exact_input_amount_out_floor(1000, 50, 100_000, 250),
            Err(MathError::ExceedsVirtualReserves)
        );
    }

    #[test]
    fn exact_input_reports_overflow_instead_of_wrapping() {
        assert_eq!(
            exact_input_amount_out_floor(u128::MAX, 1000, 100_000, 1),
            Err(MathError::Overflow)
        );
    }

    #[test]
    fn reverse_exact_input_reports_overflow_instead_of_wrapping() {
        assert_eq!(
            exact_input_amount_out_floor(u128::MAX, 100, 100_000, 1),
            Err(MathError::Overflow)
        );
    }
}
