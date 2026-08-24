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
    /// Gross debit from the trader.
    pub amount_in: u128,
    /// Amount used by the constant-product quote.
    pub effective_amount_in: u128,
    /// Collateral output before a sell-side protocol fee.
    pub raw_amount_out: u128,
    /// Amount received by the trader after a possible sell-side protocol fee.
    pub amount_out: u128,
    /// Fee transferred atomically to the collateral treasury.
    pub protocol_fee: u128,
    /// Buys charge the collateral input; sells charge the collateral output.
    pub protocol_fee_on_output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    Closed,
    ZeroAmount,
    /// A non-zero input would produce no output after conservative integer rounding.
    /// Rejecting this avoids accepting a value-destructive no-op trade.
    OutputTooSmall,
    /// A requested exact output would require no input after conservative rounding.
    /// Rejecting this prevents a zero-cost transfer.
    InputTooSmall,
    FeeAboveDenominator,
    PoolFeeUnsupported,
    InputConsumedByFees,
    ExactOutputRequiresCollateralInput,
    Arithmetic,
    SlippageExceeded,
    InsufficientRealOutputReserve,
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Closed => "the pool is closed",
            Self::ZeroAmount => "swap input or requested output must be non-zero",
            Self::OutputTooSmall => "the input is too small to produce one output unit",
            Self::InputTooSmall => "the requested output does not require one input unit",
            Self::FeeAboveDenominator => "protocol fee exceeds 10,000 basis points",
            Self::PoolFeeUnsupported => "retained pool fees are not supported by this launchpad",
            Self::InputConsumedByFees => "fees consume the entire input amount",
            Self::ExactOutputRequiresCollateralInput => {
                "exact-output swaps are supported only for collateral-input buys"
            }
            Self::Arithmetic => "the swap would overflow the supported arithmetic range",
            Self::SlippageExceeded => "the swap would exceed its slippage limit",
            Self::InsufficientRealOutputReserve => {
                "the pool lacks the real reserve for this output"
            }
        };
        f.write_str(message)
    }
}

impl std::error::Error for SwapError {}

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
        if pool_fee_bps != 0 {
            return Err(SwapError::PoolFeeUnsupported);
        }
        validate_fee_rate(protocol_fee_bps)?;
        let is_buy = token_in == TokenSide::Token1;
        let protocol_fee = if is_buy {
            fee_ceil(amount_in, protocol_fee_bps)?
        } else {
            0
        };
        let effective_input = amount_in
            .checked_sub(protocol_fee)
            .ok_or(SwapError::Arithmetic)?;
        if effective_input == 0 {
            return Err(SwapError::InputConsumedByFees);
        }
        let (virtual_in, virtual_out, real_in, real_out) = self.reserves(token_in);
        let amount_out = curve_math::exact_input_amount_out_floor(
            virtual_in,
            virtual_out,
            self.k,
            effective_input,
        )
        .map_err(|_| SwapError::Arithmetic)?;
        if amount_out == 0 {
            return Err(SwapError::OutputTooSmall);
        }
        let (recipient_amount_out, protocol_fee, protocol_fee_on_output) = if is_buy {
            (amount_out, protocol_fee, false)
        } else {
            let fee = fee_ceil(amount_out, protocol_fee_bps)?;
            let net = amount_out.checked_sub(fee).ok_or(SwapError::Arithmetic)?;
            if net == 0 {
                return Err(SwapError::OutputTooSmall);
            }
            (net, fee, true)
        };
        if recipient_amount_out < min_amount_out {
            return Err(SwapError::SlippageExceeded);
        }
        if amount_out > real_out {
            return Err(SwapError::InsufficientRealOutputReserve);
        }
        let new_virtual_in = virtual_in
            .checked_add(effective_input)
            .ok_or(SwapError::Arithmetic)?;
        let new_virtual_out = virtual_out
            .checked_sub(amount_out)
            .ok_or(SwapError::Arithmetic)?;
        self.set_reserves(
            token_in,
            new_virtual_in,
            new_virtual_out,
            real_in
                .checked_add(effective_input)
                .ok_or(SwapError::Arithmetic)?,
            real_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
        );
        self.close_if_depleted();
        Ok(SwapOutcome {
            amount_in,
            effective_amount_in: effective_input,
            raw_amount_out: amount_out,
            amount_out: recipient_amount_out,
            protocol_fee,
            protocol_fee_on_output,
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
        if token_in != TokenSide::Token1 {
            return Err(SwapError::ExactOutputRequiresCollateralInput);
        }
        if pool_fee_bps != 0 {
            return Err(SwapError::PoolFeeUnsupported);
        }
        if amount_out == 0 {
            return Err(SwapError::ZeroAmount);
        }
        let (virtual_in, virtual_out, real_in, real_out) = self.reserves(token_in);
        if amount_out > real_out {
            return Err(SwapError::InsufficientRealOutputReserve);
        }
        let required_effective_input =
            curve_math::exact_output_amount_in_ceil(virtual_in, virtual_out, self.k, amount_out)
                .map_err(|_| SwapError::Arithmetic)?;
        if required_effective_input == 0 {
            return Err(SwapError::InputTooSmall);
        }
        validate_fee_rate(protocol_fee_bps)?;
        if protocol_fee_bps == 10_000 {
            return Err(SwapError::InputConsumedByFees);
        }
        let amount_in = mul_div_ceil(
            required_effective_input,
            10_000,
            u128::from(
                10_000_u16
                    .checked_sub(protocol_fee_bps)
                    .ok_or(SwapError::Arithmetic)?,
            ),
        )?;
        let protocol_fee = fee_ceil(amount_in, protocol_fee_bps)?;
        let effective_input = amount_in
            .checked_sub(protocol_fee)
            .ok_or(SwapError::Arithmetic)?;
        if amount_in > max_amount_in {
            return Err(SwapError::SlippageExceeded);
        }
        let new_virtual_in = virtual_in
            .checked_add(effective_input)
            .ok_or(SwapError::Arithmetic)?;
        let new_virtual_out = virtual_out
            .checked_sub(amount_out)
            .ok_or(SwapError::Arithmetic)?;
        self.set_reserves(
            token_in,
            new_virtual_in,
            new_virtual_out,
            real_in
                .checked_add(effective_input)
                .ok_or(SwapError::Arithmetic)?,
            real_out
                .checked_sub(amount_out)
                .ok_or(SwapError::Arithmetic)?,
        );
        self.close_if_depleted();
        Ok(SwapOutcome {
            amount_in,
            effective_amount_in: effective_input,
            raw_amount_out: amount_out,
            amount_out,
            protocol_fee,
            protocol_fee_on_output: false,
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

fn validate_fee_rate(protocol_fee_bps: u16) -> Result<(), SwapError> {
    if protocol_fee_bps > 10_000 {
        Err(SwapError::FeeAboveDenominator)
    } else {
        Ok(())
    }
}

fn fee_ceil(amount: u128, protocol_fee_bps: u16) -> Result<u128, SwapError> {
    mul_div_ceil(amount, u128::from(protocol_fee_bps), 10_000)
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
        assert_eq!(outcome.protocol_fee, 0);
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
    fn protocol_fee_is_collateral_input_on_buy_and_collateral_output_on_sell() {
        let mut buy_pool = Pool::create(800, 100, 1_000, 100, None, None).expect("valid pool");
        let buy = buy_pool
            .swap_exact_input(TokenSide::Token1, 25, 180, 0, 1_000, 1)
            .expect("buy succeeds");
        assert_eq!((buy.effective_amount_in, buy.protocol_fee), (22, 3));
        assert_eq!((buy.raw_amount_out, buy.amount_out), (180, 180));
        assert!(!buy.protocol_fee_on_output);
        assert_eq!(buy_pool.real_reserve1, 122);

        let mut sell_pool = Pool::create(800, 100, 1_000, 100, None, None).expect("valid pool");
        let sell = sell_pool
            .swap_exact_input(TokenSide::Token0, 250, 18, 0, 1_000, 1)
            .expect("sell succeeds");
        assert_eq!(
            (sell.raw_amount_out, sell.protocol_fee, sell.amount_out),
            (20, 2, 18)
        );
        assert!(sell.protocol_fee_on_output);
        assert_eq!(
            sell_pool.real_reserve1, 80,
            "the fee leaves the collateral reserve"
        );
    }

    #[test]
    fn exact_output_swaps_token1_for_token0_with_the_fee_inside_max_input() {
        let mut pool = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
        let outcome = pool
            .swap_exact_output(TokenSide::Token1, 200, 28, 0, 1_000, 1)
            .expect("swap succeeds");

        assert_eq!(outcome.amount_in, 28);
        assert_eq!(outcome.amount_out, 200);
        assert_eq!(outcome.protocol_fee, 3);
        assert_eq!(pool.virtual_reserve0, 800);
        assert_eq!(pool.virtual_reserve1, 125);
        assert_eq!(pool.real_reserve0, 600);
        assert_eq!(pool.real_reserve1, 125);
    }

    #[test]
    fn later_quotes_continue_to_use_the_creation_time_k() {
        let mut pool = Pool::create(1_000, 1_000, 1_000, 1_000, None, None).expect("valid pool");

        pool.swap_exact_input(TokenSide::Token0, 100, 0, 0, 0, 1)
            .expect("first swap succeeds");
        // The first integer-rounded swap leaves the live reserve product at
        // 1,001,000. RFP-015 fixes the pricing invariant at creation-time k,
        // so the next 100-unit input pays 76 rather than the 75 a live-product
        // quote would produce.
        let outcome = pool
            .swap_exact_input(TokenSide::Token0, 100, 0, 0, 0, 1)
            .expect("second swap succeeds");

        assert_eq!(pool.k, 1_000_000);
        assert_eq!(outcome.amount_out, 76);
    }

    #[test]
    fn exact_output_rejects_a_zero_cost_quote_without_mutating_the_pool() {
        let mut pool = Pool::create(1, 74_513, 1, 1_709, None, None).expect("valid pool");
        pool.swap_exact_input(TokenSide::Token0, u64::MAX.into(), 0, 0, 0, 1)
            .expect("first swap succeeds");
        let before = pool.clone();

        assert_eq!(
            pool.swap_exact_output(TokenSide::Token1, 1, 0, 0, 0, 1),
            Err(SwapError::InputTooSmall)
        );
        assert_eq!(pool, before);
    }

    #[test]
    fn exact_output_uses_the_smallest_fee_inclusive_gross_input() {
        for fee_bps in [1, 2_500, 9_999] {
            let mut pool = Pool::create(800, 100, 1000, 100, None, None).expect("valid pool");
            let outcome = pool
                .swap_exact_output(TokenSide::Token1, 100, u128::MAX, 0, fee_bps, 1)
                .expect("swap succeeds");
            let required_net = 12;
            assert_eq!(
                outcome.effective_amount_in,
                outcome.amount_in - outcome.protocol_fee
            );
            assert!(outcome.effective_amount_in >= required_net);
            let previous = outcome.amount_in.checked_sub(1).expect("positive input");
            let previous_effective = previous - fee_ceil(previous, fee_bps).expect("fee fits");
            assert!(previous_effective < required_net);
        }
    }

    #[test]
    fn exact_output_is_the_smallest_zero_fee_input_that_reaches_the_requested_amount() {
        for amount_out in [1, 100, 200, 799] {
            let mut quoted = Pool::create(800, 100, 1_000, 100, None, None).expect("valid pool");
            let outcome = quoted
                .swap_exact_output(TokenSide::Token1, amount_out, u128::MAX, 0, 0, 1)
                .expect("exact-output quote succeeds");

            let mut at_quote = Pool::create(800, 100, 1_000, 100, None, None).expect("valid pool");
            let produced = at_quote
                .swap_exact_input(TokenSide::Token1, outcome.amount_in, 0, 0, 0, 1)
                .expect("the quoted input is executable");
            assert!(produced.amount_out >= amount_out);

            let mut below_quote =
                Pool::create(800, 100, 1_000, 100, None, None).expect("valid pool");
            if outcome.amount_in > 1 {
                let previous = below_quote
                    .swap_exact_input(TokenSide::Token1, outcome.amount_in - 1, 0, 0, 0, 1)
                    .expect("one less input remains a positive trade");
                assert!(previous.amount_out < amount_out);
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
    fn exact_input_rejects_a_nonzero_trade_that_rounds_to_zero_output() {
        let mut pool = Pool::create(800, 100, 1_000, 1_000, None, None).expect("valid pool");
        let before = pool.clone();

        assert_eq!(
            pool.swap_exact_input(TokenSide::Token0, 1, 0, 0, 0, 1),
            Err(SwapError::OutputTooSmall)
        );
        assert_eq!(pool, before, "a rejected dust trade must be atomic");
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
