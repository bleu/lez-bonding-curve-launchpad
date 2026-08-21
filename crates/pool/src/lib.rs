//! The bounded-pool state machine behind the neutral AMM interface.
//!
//! Knows about ordered real and virtual reserves, fees, slippage, expiry, and lifecycle.
//! Knows nothing about `AccountWithMetadata`, PDAs, or
//! `ChainedCall`, which is what lets the solvency property test run against a state
//! machine instead of hand-built account fixtures.
//!
//! Launch-specific allocation and vocabulary belong to the factory adapter.

use borsh::{BorshDeserialize, BorshSerialize};

/// Why a pool could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    /// A virtual reserve at or above [`VIRTUAL_RESERVE_BOUND`] would let `k` leave u128.
    VirtualReserveAboveBound,
    /// Zero virtual reserves make `k` zero and cannot price a swap.
    VirtualReserveZero,
}

// Hand-written: the dependency rules keep `thiserror` out of this crate.
impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VirtualReserveAboveBound => {
                write!(
                    f,
                    "a virtual reserve at or above 2^64 would let k leave u128"
                )
            }
            Self::VirtualReserveZero => {
                write!(f, "virtual reserves must be non-zero")
            }
        }
    }
}

impl std::error::Error for CreateError {}

/// Both virtual reserves stay below 2^64 so `k = Vt * Vc` always fits u128.
/// The full argument is `docs/adr/0004`.
pub const VIRTUAL_RESERVE_BOUND: u128 = 1 << 64;

/// A bounded constant-product pool with ordered token roles.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Pool {
    pub virtual_reserve0: u128,
    pub virtual_reserve1: u128,
    pub k: u128,
    pub real_reserve0: u128,
    pub real_reserve1: u128,
    pub close_timestamp: Option<u64>,
    pub open: bool,
    pub retired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSide {
    Token0,
    Token1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapOutcome {
    pub amount_in: u128,
    pub amount_out: u128,
    pub fee: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    Closed,
    ZeroAmount,
    FeeAboveDenominator,
    Arithmetic,
    SlippageExceeded,
    InsufficientRealOutputReserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawError {
    StillOpen,
    Retired,
}

impl Pool {
    /// Creates an ordered pool and fixes `k` for its lifetime.
    pub fn create(
        token0_amount: u128,
        token1_amount: u128,
        virtual_reserve0: u128,
        virtual_reserve1: u128,
        close_timestamp: Option<u64>,
    ) -> Result<Self, CreateError> {
        if virtual_reserve0 >= VIRTUAL_RESERVE_BOUND || virtual_reserve1 >= VIRTUAL_RESERVE_BOUND {
            return Err(CreateError::VirtualReserveAboveBound);
        }
        if virtual_reserve0 == 0 || virtual_reserve1 == 0 {
            return Err(CreateError::VirtualReserveZero);
        }
        let k = virtual_reserve0
            .checked_mul(virtual_reserve1)
            .expect("both virtual reserves are below 2^64, so the product fits u128");
        Ok(Self {
            virtual_reserve0,
            virtual_reserve1,
            k,
            real_reserve0: token0_amount,
            real_reserve1: token1_amount,
            close_timestamp,
            open: true,
            retired: false,
        })
    }

    pub fn swap_exact_input(
        &mut self,
        token_in: TokenSide,
        amount_in: u128,
        min_amount_out: u128,
        fee_bps: u16,
        now: u64,
    ) -> Result<SwapOutcome, SwapError> {
        self.ensure_swappable(now)?;
        if amount_in == 0 {
            return Err(SwapError::ZeroAmount);
        }
        if fee_bps > 10_000 {
            return Err(SwapError::FeeAboveDenominator);
        }
        let fee = amount_in
            .checked_mul(u128::from(fee_bps))
            .ok_or(SwapError::Arithmetic)?
            .checked_div(10_000)
            .ok_or(SwapError::Arithmetic)?;
        let net_input = amount_in.checked_sub(fee).ok_or(SwapError::Arithmetic)?;
        let (virtual_in, virtual_out, real_in, real_out) = self.reserves(token_in);
        let amount_out =
            curve_math::exact_input_amount_out_floor(virtual_in, virtual_out, self.k, net_input)
                .map_err(|_| SwapError::Arithmetic)?;
        if amount_out < min_amount_out {
            return Err(SwapError::SlippageExceeded);
        }
        if amount_out > real_out {
            return Err(SwapError::InsufficientRealOutputReserve);
        }
        self.set_reserves(
            token_in,
            virtual_in
                .checked_add(net_input)
                .ok_or(SwapError::Arithmetic)?,
            virtual_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
            real_in
                .checked_add(net_input)
                .ok_or(SwapError::Arithmetic)?,
            real_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
        );
        Ok(SwapOutcome {
            amount_in,
            amount_out,
            fee,
        })
    }

    pub fn swap_exact_output(
        &mut self,
        token_in: TokenSide,
        amount_out: u128,
        max_amount_in: u128,
        fee_bps: u16,
        now: u64,
    ) -> Result<SwapOutcome, SwapError> {
        self.ensure_swappable(now)?;
        if amount_out == 0 {
            return Err(SwapError::ZeroAmount);
        }
        if fee_bps >= 10_000 {
            return Err(SwapError::FeeAboveDenominator);
        }
        let (virtual_in, virtual_out, real_in, real_out) = self.reserves(token_in);
        if amount_out > real_out {
            return Err(SwapError::InsufficientRealOutputReserve);
        }
        let net_input =
            curve_math::exact_output_amount_in_ceil(virtual_in, virtual_out, self.k, amount_out)
                .map_err(|_| SwapError::Arithmetic)?;
        let fee = mul_div_floor(
            net_input.checked_sub(1).ok_or(SwapError::Arithmetic)?,
            u128::from(fee_bps),
            u128::from(
                10_000_u16
                    .checked_sub(fee_bps)
                    .ok_or(SwapError::Arithmetic)?,
            ),
        )?;
        let amount_in = net_input.checked_add(fee).ok_or(SwapError::Arithmetic)?;
        if amount_in > max_amount_in {
            return Err(SwapError::SlippageExceeded);
        }
        self.set_reserves(
            token_in,
            virtual_in
                .checked_add(net_input)
                .ok_or(SwapError::Arithmetic)?,
            virtual_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
            real_in
                .checked_add(net_input)
                .ok_or(SwapError::Arithmetic)?,
            real_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
        );
        Ok(SwapOutcome {
            amount_in,
            amount_out,
            fee,
        })
    }

    pub fn close_pool(&mut self) {
        if !self.retired {
            self.open = false;
        }
    }

    pub fn withdraw_reserves(&mut self, now: u64) -> Result<(u128, u128), WithdrawError> {
        if self.retired {
            return Err(WithdrawError::Retired);
        }
        let expired = self.close_timestamp.is_some_and(|close| now >= close);
        if self.open && !expired {
            return Err(WithdrawError::StillOpen);
        }
        let reserves = (self.real_reserve0, self.real_reserve1);
        self.real_reserve0 = 0;
        self.real_reserve1 = 0;
        self.open = false;
        self.retired = true;
        Ok(reserves)
    }

    fn ensure_swappable(&self, now: u64) -> Result<(), SwapError> {
        if !self.open || self.retired || self.close_timestamp.is_some_and(|close| now >= close) {
            Err(SwapError::Closed)
        } else {
            Ok(())
        }
    }

    fn reserves(&self, token_in: TokenSide) -> (u128, u128, u128, u128) {
        match token_in {
            TokenSide::Token0 => (
                self.virtual_reserve0,
                self.virtual_reserve1,
                self.real_reserve0,
                self.real_reserve1,
            ),
            TokenSide::Token1 => (
                self.virtual_reserve1,
                self.virtual_reserve0,
                self.real_reserve1,
                self.real_reserve0,
            ),
        }
    }

    fn set_reserves(
        &mut self,
        token_in: TokenSide,
        virtual_in: u128,
        virtual_out: u128,
        real_in: u128,
        real_out: u128,
    ) {
        match token_in {
            TokenSide::Token0 => {
                self.virtual_reserve0 = virtual_in;
                self.virtual_reserve1 = virtual_out;
                self.real_reserve0 = real_in;
                self.real_reserve1 = real_out;
            }
            TokenSide::Token1 => {
                self.virtual_reserve1 = virtual_in;
                self.virtual_reserve0 = virtual_out;
                self.real_reserve1 = real_in;
                self.real_reserve0 = real_out;
            }
        }
    }
}

fn mul_div_floor(value: u128, multiplier: u128, divisor: u128) -> Result<u128, SwapError> {
    let quotient = value.checked_div(divisor).ok_or(SwapError::Arithmetic)?;
    let remainder = value.checked_rem(divisor).ok_or(SwapError::Arithmetic)?;
    let whole = quotient
        .checked_mul(multiplier)
        .ok_or(SwapError::Arithmetic)?;
    let fraction = remainder
        .checked_mul(multiplier)
        .ok_or(SwapError::Arithmetic)?
        .checked_div(divisor)
        .ok_or(SwapError::Arithmetic)?;
    whole.checked_add(fraction).ok_or(SwapError::Arithmetic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_pool_records_both_real_reserves_and_optional_close_time() {
        let pool = Pool::create(800, 25, 1000, 100, Some(42)).expect("valid pool");
        assert_eq!(pool.real_reserve0, 800);
        assert_eq!(pool.real_reserve1, 25);
        assert_eq!(pool.virtual_reserve0, 1000);
        assert_eq!(pool.virtual_reserve1, 100);
        assert_eq!(pool.close_timestamp, Some(42));
        assert_eq!(pool.k, 100_000);
        assert!(pool.open);
        assert!(!pool.retired);
    }

    #[test]
    fn exact_input_swaps_token0_for_token1_through_the_public_pool_api() {
        let mut pool = Pool::create(800, 100, 1000, 100, None).expect("valid pool");
        let outcome = pool
            .swap_exact_input(TokenSide::Token0, 250, 20, 0, 1)
            .expect("swap succeeds");

        assert_eq!(outcome.amount_in, 250);
        assert_eq!(outcome.amount_out, 20);
        assert_eq!(outcome.fee, 0);
        assert_eq!(pool.virtual_reserve0, 1250);
        assert_eq!(pool.virtual_reserve1, 80);
        assert_eq!(pool.real_reserve0, 1050);
        assert_eq!(pool.real_reserve1, 80);
    }

    #[test]
    fn exact_input_fee_is_removed_from_whichever_token_is_input() {
        let mut token0_in = Pool::create(800, 100, 1000, 100, None).expect("valid pool");
        let forward = token0_in
            .swap_exact_input(TokenSide::Token0, 250, 18, 1_000, 1)
            .expect("token0 input succeeds");
        assert_eq!((forward.fee, forward.amount_out), (25, 18));
        assert_eq!(token0_in.real_reserve0, 1025);

        let mut token1_in = Pool::create(800, 100, 1000, 100, None).expect("valid pool");
        let reverse = token1_in
            .swap_exact_input(TokenSide::Token1, 25, 186, 1_000, 1)
            .expect("token1 input succeeds");
        assert_eq!((reverse.fee, reverse.amount_out), (2, 186));
        assert_eq!(token1_in.real_reserve1, 123);
    }

    #[test]
    fn exact_output_swaps_token1_for_token0_with_the_fee_inside_max_input() {
        let mut pool = Pool::create(800, 100, 1000, 100, None).expect("valid pool");
        let outcome = pool
            .swap_exact_output(TokenSide::Token1, 200, 27, 1_000, 1)
            .expect("swap succeeds");

        assert_eq!(outcome.amount_in, 27);
        assert_eq!(outcome.amount_out, 200);
        assert_eq!(outcome.fee, 2);
        assert_eq!(pool.virtual_reserve0, 800);
        assert_eq!(pool.virtual_reserve1, 125);
        assert_eq!(pool.real_reserve0, 600);
        assert_eq!(pool.real_reserve1, 125);
    }

    #[test]
    fn exact_output_uses_the_smallest_fee_inclusive_gross_input() {
        for fee_bps in [1, 2_500, 9_999] {
            let mut pool = Pool::create(800, 100, 1000, 100, None).expect("valid pool");
            let outcome = pool
                .swap_exact_output(TokenSide::Token1, 100, u128::MAX, fee_bps, 1)
                .expect("swap succeeds");
            let required_net = 12;
            let charged = mul_div_floor(outcome.amount_in, u128::from(fee_bps), 10_000)
                .expect("small fee arithmetic");
            let net = outcome.amount_in.checked_sub(charged).expect("fee fits");
            assert!(net >= required_net);
            let previous = outcome.amount_in.checked_sub(1).expect("positive input");
            let previous_fee =
                mul_div_floor(previous, u128::from(fee_bps), 10_000).expect("small fee arithmetic");
            let previous_net = previous.checked_sub(previous_fee).expect("fee fits");
            assert!(previous_net < required_net);
        }
    }

    #[test]
    fn expiry_allows_full_withdrawal_without_a_close_transaction_and_retires_the_pool() {
        let mut pool = Pool::create(800, 25, 1000, 100, Some(42)).expect("valid pool");
        assert_eq!(pool.withdraw_reserves(42), Ok((800, 25)));
        assert_eq!(pool.real_reserve0, 0);
        assert_eq!(pool.real_reserve1, 0);
        assert!(!pool.open);
        assert!(pool.retired);
        assert_eq!(pool.withdraw_reserves(43), Err(WithdrawError::Retired));
    }

    #[test]
    fn swaps_are_rejected_at_the_close_timestamp_without_mutating_reserves() {
        let mut pool = Pool::create(800, 100, 1000, 100, Some(42)).expect("valid pool");
        let before = pool.clone();
        assert_eq!(
            pool.swap_exact_input(TokenSide::Token0, 250, 20, 0, 42),
            Err(SwapError::Closed)
        );
        assert_eq!(pool, before);
    }

    #[test]
    fn a_swap_cannot_exceed_the_selected_real_output_reserve() {
        let mut pool = Pool::create(800, 10, 1000, 100, None).expect("valid pool");
        let before = pool.clone();
        assert_eq!(
            pool.swap_exact_input(TokenSide::Token0, 250, 0, 0, 1),
            Err(SwapError::InsufficientRealOutputReserve)
        );
        assert_eq!(pool, before);
        assert!(pool.open, "reserve exhaustion must not auto-close the pool");
    }

    #[test]
    fn manual_close_allows_full_withdrawal_for_a_pool_without_expiry() {
        let mut pool = Pool::create(800, 25, 1000, 100, None).expect("valid pool");
        assert_eq!(pool.withdraw_reserves(1), Err(WithdrawError::StillOpen));
        pool.close_pool();
        assert_eq!(pool.withdraw_reserves(1), Ok((800, 25)));
        assert!(pool.retired);
    }

    #[test]
    fn create_bounds_both_virtual_reserves_below_two_to_the_64() {
        assert_eq!(
            Pool::create(800, 0, 1 << 64, 100, None),
            Err(CreateError::VirtualReserveAboveBound)
        );
        assert_eq!(
            Pool::create(800, 0, 1000, 1 << 64, None),
            Err(CreateError::VirtualReserveAboveBound)
        );
        assert!(Pool::create(800, 0, (1 << 64) - 1, (1 << 64) - 1, None).is_ok());
    }

    #[test]
    fn create_requires_both_virtual_reserves_to_price_the_pool() {
        assert_eq!(
            Pool::create(800, 200, 1000, 0, None),
            Err(CreateError::VirtualReserveZero)
        );
        assert_eq!(
            Pool::create(800, 200, 0, 100, None),
            Err(CreateError::VirtualReserveZero)
        );
    }

    #[test]
    fn a_pool_survives_a_borsh_round_trip() {
        let pool = Pool::create(800, 200, 1000, 100, Some(42)).expect("valid pool");
        let bytes = borsh::to_vec(&pool).expect("pool serialises");
        assert_eq!(Pool::try_from_slice(&bytes).expect("bytes parse"), pool);
    }
}
