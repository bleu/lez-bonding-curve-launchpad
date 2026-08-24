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
    /// A configured depletion side must begin with a positive real reserve.
    DepletionReserveZero,
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
            Self::DepletionReserveZero => {
                write!(
                    f,
                    "the configured depletion-side real reserve must be non-zero"
                )
            }
        }
    }
}

impl std::error::Error for CreateError {}

/// Both initial virtual reserves stay below 2^64 so creation-time `k` fits u128.
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
    pub close_on_depletion: Option<TokenSide>,
    pub lifecycle: PoolLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum TokenSide {
    Token0,
    Token1,
}

/// The persisted lifecycle. Timestamp expiry is evaluated by [`Pool::effective_lifecycle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum PoolLifecycle {
    Open,
    Closed,
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapOutcome {
    /// Gross debit from the trader, including both fee portions.
    pub amount_in: u128,
    /// Amount used by the constant-product quote.
    pub effective_amount_in: u128,
    pub amount_out: u128,
    /// Fee retained in the input-side real and virtual reserves.
    pub pool_fee: u128,
    /// Fee transferred to the treasury holding for the input token.
    pub protocol_fee: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    Closed,
    ZeroAmount,
    FeeAboveDenominator,
    InputConsumedByFees,
    Arithmetic,
    SlippageExceeded,
    InsufficientRealOutputReserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawError {
    StillOpen,
    AlreadyWithdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseError {
    AlreadyClosed,
}

impl Pool {
    /// Creates an ordered pool and fixes `k` for its lifetime.
    pub fn create(
        token0_amount: u128,
        token1_amount: u128,
        virtual_reserve0: u128,
        virtual_reserve1: u128,
        close_timestamp: Option<u64>,
        close_on_depletion: Option<TokenSide>,
    ) -> Result<Self, CreateError> {
        if virtual_reserve0 >= VIRTUAL_RESERVE_BOUND || virtual_reserve1 >= VIRTUAL_RESERVE_BOUND {
            return Err(CreateError::VirtualReserveAboveBound);
        }
        if virtual_reserve0 == 0 || virtual_reserve1 == 0 {
            return Err(CreateError::VirtualReserveZero);
        }
        if matches!(close_on_depletion, Some(TokenSide::Token0)) && token0_amount == 0
            || matches!(close_on_depletion, Some(TokenSide::Token1)) && token1_amount == 0
        {
            return Err(CreateError::DepletionReserveZero);
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
            close_on_depletion,
            lifecycle: PoolLifecycle::Open,
        })
    }

    pub fn swap_exact_input(
        &mut self,
        token_in: TokenSide,
        amount_in: u128,
        min_amount_out: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
        now: u64,
    ) -> Result<SwapOutcome, SwapError> {
        self.ensure_swappable(now)?;
        if amount_in == 0 {
            return Err(SwapError::ZeroAmount);
        }
        let (effective_input, pool_fee, protocol_fee) =
            split_input(amount_in, pool_fee_bps, protocol_fee_bps)?;
        let (virtual_in, virtual_out, real_in, real_out) = self.reserves(token_in);
        let current_product = virtual_in
            .checked_mul(virtual_out)
            .ok_or(SwapError::Arithmetic)?;
        let amount_out = curve_math::exact_input_amount_out_floor(
            virtual_in,
            virtual_out,
            current_product,
            effective_input,
        )
        .map_err(|_| SwapError::Arithmetic)?;
        if amount_out < min_amount_out {
            return Err(SwapError::SlippageExceeded);
        }
        if amount_out > real_out {
            return Err(SwapError::InsufficientRealOutputReserve);
        }
        let retained_input = effective_input
            .checked_add(pool_fee)
            .ok_or(SwapError::Arithmetic)?;
        let new_virtual_in = virtual_in
            .checked_add(retained_input)
            .ok_or(SwapError::Arithmetic)?;
        let new_virtual_out = virtual_out
            .checked_sub(amount_out)
            .ok_or(SwapError::Arithmetic)?;
        new_virtual_in
            .checked_mul(new_virtual_out)
            .ok_or(SwapError::Arithmetic)?;
        self.set_reserves(
            token_in,
            new_virtual_in,
            new_virtual_out,
            real_in
                .checked_add(retained_input)
                .ok_or(SwapError::Arithmetic)?,
            real_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
        );
        self.close_if_depleted();
        Ok(SwapOutcome {
            amount_in,
            effective_amount_in: effective_input,
            amount_out,
            pool_fee,
            protocol_fee,
        })
    }

    pub fn swap_exact_output(
        &mut self,
        token_in: TokenSide,
        amount_out: u128,
        max_amount_in: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
        now: u64,
    ) -> Result<SwapOutcome, SwapError> {
        self.ensure_swappable(now)?;
        if amount_out == 0 {
            return Err(SwapError::ZeroAmount);
        }
        let (virtual_in, virtual_out, real_in, real_out) = self.reserves(token_in);
        if amount_out > real_out {
            return Err(SwapError::InsufficientRealOutputReserve);
        }
        let current_product = virtual_in
            .checked_mul(virtual_out)
            .ok_or(SwapError::Arithmetic)?;
        let required_effective_input = curve_math::exact_output_amount_in_ceil(
            virtual_in,
            virtual_out,
            current_product,
            amount_out,
        )
        .map_err(|_| SwapError::Arithmetic)?;
        let total_fee_bps = validate_fee_rates(pool_fee_bps, protocol_fee_bps)?;
        if total_fee_bps == 10_000 {
            return Err(SwapError::InputConsumedByFees);
        }
        let amount_in = mul_div_ceil(
            required_effective_input,
            10_000,
            u128::from(
                10_000_u16
                    .checked_sub(total_fee_bps)
                    .ok_or(SwapError::Arithmetic)?,
            ),
        )?;
        let (effective_input, pool_fee, protocol_fee) =
            split_input(amount_in, pool_fee_bps, protocol_fee_bps)?;
        if amount_in > max_amount_in {
            return Err(SwapError::SlippageExceeded);
        }
        let retained_input = effective_input
            .checked_add(pool_fee)
            .ok_or(SwapError::Arithmetic)?;
        let new_virtual_in = virtual_in
            .checked_add(retained_input)
            .ok_or(SwapError::Arithmetic)?;
        let new_virtual_out = virtual_out
            .checked_sub(amount_out)
            .ok_or(SwapError::Arithmetic)?;
        new_virtual_in
            .checked_mul(new_virtual_out)
            .ok_or(SwapError::Arithmetic)?;
        self.set_reserves(
            token_in,
            new_virtual_in,
            new_virtual_out,
            real_in
                .checked_add(retained_input)
                .ok_or(SwapError::Arithmetic)?,
            real_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
        );
        self.close_if_depleted();
        Ok(SwapOutcome {
            amount_in,
            effective_amount_in: effective_input,
            amount_out,
            pool_fee,
            protocol_fee,
        })
    }

    pub fn close_pool(&mut self, now: u64) -> Result<(), CloseError> {
        if self.effective_lifecycle(now) != PoolLifecycle::Open {
            return Err(CloseError::AlreadyClosed);
        }
        self.lifecycle = PoolLifecycle::Closed;
        Ok(())
    }

    pub fn withdraw_reserves(&mut self, now: u64) -> Result<(u128, u128), WithdrawError> {
        if self.lifecycle == PoolLifecycle::Withdrawn {
            return Err(WithdrawError::AlreadyWithdrawn);
        }
        if self.effective_lifecycle(now) == PoolLifecycle::Open {
            return Err(WithdrawError::StillOpen);
        }
        let reserves = (self.real_reserve0, self.real_reserve1);
        self.real_reserve0 = 0;
        self.real_reserve1 = 0;
        self.lifecycle = PoolLifecycle::Withdrawn;
        Ok(reserves)
    }

    /// Reports logical timestamp expiry without requiring a state write.
    #[must_use]
    pub fn effective_lifecycle(&self, now: u64) -> PoolLifecycle {
        if self.lifecycle == PoolLifecycle::Open
            && self.close_timestamp.is_some_and(|close| now >= close)
        {
            PoolLifecycle::Closed
        } else {
            self.lifecycle
        }
    }

    fn ensure_swappable(&self, now: u64) -> Result<(), SwapError> {
        if self.effective_lifecycle(now) != PoolLifecycle::Open {
            Err(SwapError::Closed)
        } else {
            Ok(())
        }
    }

    fn close_if_depleted(&mut self) {
        let depleted = match self.close_on_depletion {
            Some(TokenSide::Token0) => self.real_reserve0 == 0,
            Some(TokenSide::Token1) => self.real_reserve1 == 0,
            None => false,
        };
        if depleted {
            self.lifecycle = PoolLifecycle::Closed;
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

fn mul_div_ceil(value: u128, multiplier: u128, divisor: u128) -> Result<u128, SwapError> {
    let floor = mul_div_floor(value, multiplier, divisor)?;
    let remainder = value.checked_rem(divisor).ok_or(SwapError::Arithmetic)?;
    let fractional_numerator = remainder
        .checked_mul(multiplier)
        .ok_or(SwapError::Arithmetic)?;
    if fractional_numerator
        .checked_rem(divisor)
        .ok_or(SwapError::Arithmetic)?
        == 0
    {
        Ok(floor)
    } else {
        floor.checked_add(1).ok_or(SwapError::Arithmetic)
    }
}

fn validate_fee_rates(pool_fee_bps: u16, protocol_fee_bps: u16) -> Result<u16, SwapError> {
    let total_fee_bps = pool_fee_bps
        .checked_add(protocol_fee_bps)
        .ok_or(SwapError::FeeAboveDenominator)?;
    if total_fee_bps > 10_000 {
        Err(SwapError::FeeAboveDenominator)
    } else {
        Ok(total_fee_bps)
    }
}

fn split_input(
    gross_input: u128,
    pool_fee_bps: u16,
    protocol_fee_bps: u16,
) -> Result<(u128, u128, u128), SwapError> {
    let total_fee_bps = validate_fee_rates(pool_fee_bps, protocol_fee_bps)?;
    if total_fee_bps == 0 {
        return Ok((gross_input, 0, 0));
    }

    let combined_fee = mul_div_ceil(gross_input, u128::from(total_fee_bps), 10_000)?;
    let effective_input = gross_input
        .checked_sub(combined_fee)
        .ok_or(SwapError::Arithmetic)?;
    if effective_input == 0 {
        return Err(SwapError::InputConsumedByFees);
    }
    let protocol_fee = mul_div_floor(
        combined_fee,
        u128::from(protocol_fee_bps),
        u128::from(total_fee_bps),
    )?;
    let pool_fee = combined_fee
        .checked_sub(protocol_fee)
        .ok_or(SwapError::Arithmetic)?;
    Ok((effective_input, pool_fee, protocol_fee))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_pool_records_both_real_reserves_and_optional_close_time() {
        let pool = Pool::create(800, 25, 1000, 100, Some(42), None).expect("valid pool");
        assert_eq!(pool.real_reserve0, 800);
        assert_eq!(pool.real_reserve1, 25);
        assert_eq!(pool.virtual_reserve0, 1000);
        assert_eq!(pool.virtual_reserve1, 100);
        assert_eq!(pool.close_timestamp, Some(42));
        assert_eq!(pool.k, 100_000);
        assert_eq!(pool.lifecycle, PoolLifecycle::Open);
    }

    #[test]
    fn exact_input_swaps_token0_for_token1_through_the_public_pool_api() {
        let mut pool = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
        let outcome = pool
            .swap_exact_input(TokenSide::Token0, 250, 20, 0, 0, 1)
            .expect("swap succeeds");

        assert_eq!(outcome.amount_in, 250);
        assert_eq!(outcome.amount_out, 20);
        assert_eq!((outcome.pool_fee, outcome.protocol_fee), (0, 0));
        assert_eq!(pool.virtual_reserve0, 1250);
        assert_eq!(pool.virtual_reserve1, 80);
        assert_eq!(pool.real_reserve0, 1050);
        assert_eq!(pool.real_reserve1, 80);
    }

    #[test]
    fn final_exact_input_swap_closes_a_pool_configured_to_close_on_token0_depletion() {
        let mut pool =
            Pool::create(80, 0, 100, 100, None, Some(TokenSide::Token0)).expect("valid pool");

        let outcome = pool
            .swap_exact_input(TokenSide::Token1, 400, 80, 0, 0, 1)
            .expect("final swap succeeds");

        assert_eq!(outcome.amount_out, 80);
        assert_eq!(pool.real_reserve0, 0);
        assert_eq!(pool.effective_lifecycle(1), PoolLifecycle::Closed);
        assert_eq!(
            pool.swap_exact_input(TokenSide::Token1, 1, 0, 0, 0, 1),
            Err(SwapError::Closed)
        );
    }

    #[test]
    fn final_exact_output_swap_closes_a_pool_configured_to_close_on_token0_depletion() {
        let mut pool =
            Pool::create(80, 0, 100, 100, None, Some(TokenSide::Token0)).expect("valid pool");

        pool.swap_exact_output(TokenSide::Token1, 80, u128::MAX, 0, 0, 1)
            .expect("final swap succeeds");

        assert_eq!(pool.real_reserve0, 0);
        assert_eq!(pool.effective_lifecycle(1), PoolLifecycle::Closed);
    }

    #[test]
    fn configured_zero_depletion_reserve_is_rejected() {
        assert_eq!(
            Pool::create(0, 1, 100, 100, None, Some(TokenSide::Token0)),
            Err(CreateError::DepletionReserveZero)
        );
    }

    #[test]
    fn exact_input_pool_fee_is_retained_in_whichever_token_is_input() {
        let mut token0_in = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
        let forward = token0_in
            .swap_exact_input(TokenSide::Token0, 250, 18, 1_000, 0, 1)
            .expect("token0 input succeeds");
        assert_eq!((forward.pool_fee, forward.protocol_fee), (25, 0));
        assert_eq!(forward.amount_out, 18);
        assert_eq!(token0_in.real_reserve0, 1050);

        let mut token1_in = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
        let reverse = token1_in
            .swap_exact_input(TokenSide::Token1, 25, 180, 1_000, 0, 1)
            .expect("token1 input succeeds");
        assert_eq!((reverse.pool_fee, reverse.protocol_fee), (3, 0));
        assert_eq!(reverse.amount_out, 180);
        assert_eq!(token1_in.real_reserve1, 125);
    }

    #[test]
    fn exact_output_swaps_token1_for_token0_with_the_fee_inside_max_input() {
        let mut pool = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
        let outcome = pool
            .swap_exact_output(TokenSide::Token1, 200, 28, 1_000, 0, 1)
            .expect("swap succeeds");

        assert_eq!(outcome.amount_in, 28);
        assert_eq!(outcome.amount_out, 200);
        assert_eq!(outcome.pool_fee, 3);
        assert_eq!(outcome.protocol_fee, 0);
        assert_eq!(pool.virtual_reserve0, 800);
        assert_eq!(pool.virtual_reserve1, 128);
        assert_eq!(pool.real_reserve0, 600);
        assert_eq!(pool.real_reserve1, 128);
    }

    #[test]
    fn exact_output_uses_the_smallest_fee_inclusive_gross_input() {
        for fee_bps in [1, 2_500, 9_999] {
            let mut pool = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
            let outcome = pool
                .swap_exact_output(TokenSide::Token1, 100, u128::MAX, fee_bps, 0, 1)
                .expect("swap succeeds");
            let required_net = 12;
            let (net, _, _) =
                split_input(outcome.amount_in, fee_bps, 0).expect("small fee arithmetic");
            assert!(net >= required_net);
            let previous = outcome.amount_in.checked_sub(1).expect("positive input");
            match split_input(previous, fee_bps, 0) {
                Ok((previous_net, _, _)) => assert!(previous_net < required_net),
                Err(SwapError::InputConsumedByFees) => {}
                Err(error) => panic!("unexpected fee error: {error:?}"),
            }
        }
    }

    #[test]
    fn expiry_allows_full_withdrawal_without_a_close_transaction_and_retires_the_pool() {
        let mut pool = Pool::create(800, 25, 1000, 100, Some(42), None).expect("valid pool");
        assert_eq!(pool.withdraw_reserves(42), Ok((800, 25)));
        assert_eq!(pool.real_reserve0, 0);
        assert_eq!(pool.real_reserve1, 0);
        assert_eq!(pool.lifecycle, PoolLifecycle::Withdrawn);
        assert_eq!(
            pool.withdraw_reserves(43),
            Err(WithdrawError::AlreadyWithdrawn)
        );
    }

    #[test]
    fn swaps_are_rejected_at_the_close_timestamp_without_mutating_reserves() {
        let mut pool = Pool::create(800, 100, 1000, 100, Some(42), None).expect("valid pool");
        let before = pool.clone();
        assert_eq!(
            pool.swap_exact_input(TokenSide::Token0, 250, 20, 0, 0, 42),
            Err(SwapError::Closed)
        );
        assert_eq!(pool, before);
    }

    #[test]
    fn a_swap_cannot_exceed_the_selected_real_output_reserve() {
        let mut pool = Pool::create(800, 10, 1000, 100, None, None).expect("valid pool");
        let before = pool.clone();
        assert_eq!(
            pool.swap_exact_input(TokenSide::Token0, 250, 0, 0, 0, 1),
            Err(SwapError::InsufficientRealOutputReserve)
        );
        assert_eq!(pool, before);
        assert_eq!(
            pool.lifecycle,
            PoolLifecycle::Open,
            "unconfigured depletion must not close the pool"
        );
    }

    #[test]
    fn manual_close_allows_full_withdrawal_for_a_pool_without_expiry() {
        let mut pool = Pool::create(800, 25, 1000, 100, None, None).expect("valid pool");
        assert_eq!(pool.withdraw_reserves(1), Err(WithdrawError::StillOpen));
        assert_eq!(pool.close_pool(1), Ok(()));
        assert_eq!(pool.withdraw_reserves(1), Ok((800, 25)));
        assert_eq!(pool.lifecycle, PoolLifecycle::Withdrawn);
    }

    #[test]
    fn create_bounds_both_virtual_reserves_below_two_to_the_64() {
        assert_eq!(
            Pool::create(800, 0, 1 << 64, 100, None, None),
            Err(CreateError::VirtualReserveAboveBound)
        );
        assert_eq!(
            Pool::create(800, 0, 1000, 1 << 64, None, None),
            Err(CreateError::VirtualReserveAboveBound)
        );
        assert!(Pool::create(800, 0, (1 << 64) - 1, (1 << 64) - 1, None, None).is_ok());
    }

    #[test]
    fn create_requires_both_virtual_reserves_to_price_the_pool() {
        assert_eq!(
            Pool::create(800, 200, 1000, 0, None, None),
            Err(CreateError::VirtualReserveZero)
        );
        assert_eq!(
            Pool::create(800, 200, 0, 100, None, None),
            Err(CreateError::VirtualReserveZero)
        );
    }

    #[test]
    fn a_pool_survives_a_borsh_round_trip() {
        let pool = Pool::create(800, 200, 1000, 100, Some(42), None).expect("valid pool");
        let bytes = borsh::to_vec(&pool).expect("pool serialises");
        assert_eq!(Pool::try_from_slice(&bytes).expect("bytes parse"), pool);
    }
}
