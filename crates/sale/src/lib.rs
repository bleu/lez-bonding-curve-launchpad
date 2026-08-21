//! The sale state machine: a buy, a sell, or a close applied to the sale's own data.
//!
//! Knows about virtual reserves, the two accounting buckets, fees, slippage, and the
//! open or closed flag. Knows nothing about `AccountWithMetadata`, PDAs, or
//! `ChainedCall`, which is what lets the solvency property test run against a state
//! machine instead of hand-built account fixtures.
//!
//! GTM-514 exercises these transitions with a chain-free solvency property test.

use borsh::{BorshDeserialize, BorshSerialize};
use curve_math::{MathError, buy_tokens_out_floor, sell_collateral_out_floor};

const BASIS_POINTS_DENOMINATOR: u16 = 10_000;

/// Observable effects of an accepted buy. Account handlers use this to move the
/// participant's collateral, the treasury fee, and the dispensed tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyOutcome {
    pub gross_collateral_in: u128,
    pub effective_collateral_in: u128,
    pub pool_fee: u128,
    pub protocol_fee: u128,
    pub tokens_out: u128,
    pub auto_closed: bool,
}

/// Observable effects of an accepted sell. Both fee fields are denominated in
/// the sold token, while `collateral_out` is paid from the real reserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SellOutcome {
    pub gross_tokens_in: u128,
    pub effective_tokens_in: u128,
    pub pool_fee: u128,
    pub protocol_fee: u128,
    pub collateral_out: u128,
}

/// Why a trade could not change a sale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeError {
    SaleClosed,
    ZeroInput,
    InvalidFeeRates,
    InputConsumedByFees,
    ArithmeticOverflow,
    ExceedsVirtualReserves,
    SlippageExceeded,
    ExceedsSaleReserve,
    ExceedsRealCollateralReserve,
}

/// Why an explicit manual close could not be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseError {
    SaleClosed,
}

impl std::fmt::Display for TradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::SaleClosed => "the sale is closed",
            Self::ZeroInput => "a trade input must be nonzero",
            Self::InvalidFeeRates => "the combined fee rate exceeds 10,000 basis points",
            Self::InputConsumedByFees => "the combined fee consumes the whole input",
            Self::ArithmeticOverflow => "trade arithmetic left u128",
            Self::ExceedsVirtualReserves => "the trade exceeds the virtual reserves",
            Self::SlippageExceeded => "the trade does not meet the participant's minimum output",
            Self::ExceedsSaleReserve => "the buy exceeds the remaining sale reserve",
            Self::ExceedsRealCollateralReserve => "the sell exceeds the real collateral reserve",
        };
        f.write_str(message)
    }
}

impl std::error::Error for TradeError {}

impl std::fmt::Display for CloseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SaleClosed => f.write_str("the sale is already closed"),
        }
    }
}

impl std::error::Error for CloseError {}

impl From<MathError> for TradeError {
    fn from(error: MathError) -> Self {
        match error {
            MathError::Overflow => Self::ArithmeticOverflow,
            MathError::ExceedsVirtualReserves => Self::ExceedsVirtualReserves,
        }
    }
}

/// One token launch on a bonding curve: what pricing and solvency need,
/// and nothing chain-shaped. `curve-core` nests it in `SaleAccount` next
/// to the token pair ids and serialises that into the sale PDA.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Sale {
    /// Virtual token reserve `Vt`. Pricing only; mutable.
    pub virtual_token_reserve: u128,
    /// Virtual collateral reserve `Vc`. Pricing only; mutable.
    pub virtual_collateral_reserve: u128,
    /// The constant product, fixed at creation and never changed afterwards.
    pub k: u128,
    /// Sale reserve `D`: what the curve can still dispense.
    pub sale_reserve: u128,
    /// DEX seed reserve `R`: held back for seeding a DEX after close.
    pub dex_seed_reserve: u128,
    /// Collateral actually held, as opposed to the virtual amount used for pricing.
    pub real_collateral_reserve: u128,
    pub open: bool,
}

/// Why a sale could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    /// F2: `Vt` must exceed the sale reserve, or selling out needs infinite collateral.
    VirtualTokenReserveNotAboveSaleReserve,
    /// A virtual reserve at or above [`VIRTUAL_RESERVE_BOUND`] would let `k` leave u128.
    VirtualReserveAboveBound,
    /// A sale with nothing to dispense can never complete.
    SaleReserveZero,
    /// A zero `Vc` makes `k` zero, which prices the whole virtual token
    /// reserve at nothing.
    VirtualCollateralReserveZero,
}

// Hand-written: the dependency rules keep `thiserror` out of this crate.
impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VirtualTokenReserveNotAboveSaleReserve => {
                write!(f, "the virtual token reserve must exceed the sale reserve")
            }
            Self::VirtualReserveAboveBound => {
                write!(
                    f,
                    "a virtual reserve at or above 2^64 would let k leave u128"
                )
            }
            Self::SaleReserveZero => {
                write!(f, "a sale with nothing to dispense can never complete")
            }
            Self::VirtualCollateralReserveZero => {
                write!(
                    f,
                    "a zero virtual collateral reserve prices the curve at nothing"
                )
            }
        }
    }
}

impl std::error::Error for CreateError {}

/// Both virtual reserves stay below 2^64 so `k = Vt * Vc` always fits u128.
/// The full argument is `docs/adr/0004`.
pub const VIRTUAL_RESERVE_BOUND: u128 = 1 << 64;

impl Sale {
    /// Opens a sale. Fixes `k = Vt * Vc` for its whole life.
    pub fn create(
        sale_reserve: u128,
        dex_seed_reserve: u128,
        virtual_token_reserve: u128,
        virtual_collateral_reserve: u128,
    ) -> Result<Self, CreateError> {
        if virtual_token_reserve >= VIRTUAL_RESERVE_BOUND
            || virtual_collateral_reserve >= VIRTUAL_RESERVE_BOUND
        {
            return Err(CreateError::VirtualReserveAboveBound);
        }
        if sale_reserve == 0 {
            return Err(CreateError::SaleReserveZero);
        }
        if virtual_collateral_reserve == 0 {
            return Err(CreateError::VirtualCollateralReserveZero);
        }
        if virtual_token_reserve <= sale_reserve {
            return Err(CreateError::VirtualTokenReserveNotAboveSaleReserve);
        }
        let k = virtual_token_reserve
            .checked_mul(virtual_collateral_reserve)
            .expect("both virtual reserves are below 2^64, so the product fits u128");
        Ok(Self {
            virtual_token_reserve,
            virtual_collateral_reserve,
            k,
            sale_reserve,
            dex_seed_reserve,
            real_collateral_reserve: 0,
            open: true,
        })
    }

    /// Buys tokens with a gross collateral debit.
    ///
    /// The combined input fee is rounded up once. Its protocol share leaves for
    /// the treasury; its pool share remains in the real and virtual collateral
    /// reserves. Pricing uses the current reserve product so rounding surplus can
    /// never be consumed by a later trade.
    pub fn buy(
        &mut self,
        gross_collateral_in: u128,
        min_tokens_out: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
    ) -> Result<BuyOutcome, TradeError> {
        if !self.open {
            return Err(TradeError::SaleClosed);
        }
        if gross_collateral_in == 0 {
            return Err(TradeError::ZeroInput);
        }

        let (effective_collateral_in, pool_fee, protocol_fee) =
            split_input(gross_collateral_in, pool_fee_bps, protocol_fee_bps)?;
        let old_product = self
            .virtual_token_reserve
            .checked_mul(self.virtual_collateral_reserve)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let tokens_out = buy_tokens_out_floor(
            self.virtual_token_reserve,
            self.virtual_collateral_reserve,
            old_product,
            effective_collateral_in,
        )?;
        if tokens_out < min_tokens_out {
            return Err(TradeError::SlippageExceeded);
        }
        if tokens_out > self.sale_reserve {
            return Err(TradeError::ExceedsSaleReserve);
        }

        let virtual_token_reserve = self
            .virtual_token_reserve
            .checked_sub(tokens_out)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let priced_collateral_reserve = self
            .virtual_collateral_reserve
            .checked_add(effective_collateral_in)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let virtual_collateral_reserve = priced_collateral_reserve
            .checked_add(pool_fee)
            .ok_or(TradeError::ArithmeticOverflow)?;
        virtual_token_reserve
            .checked_mul(virtual_collateral_reserve)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let sale_reserve = self
            .sale_reserve
            .checked_sub(tokens_out)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let collateral_retained = effective_collateral_in
            .checked_add(pool_fee)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let real_collateral_reserve = self
            .real_collateral_reserve
            .checked_add(collateral_retained)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let auto_closed = sale_reserve == 0;

        self.virtual_token_reserve = virtual_token_reserve;
        self.virtual_collateral_reserve = virtual_collateral_reserve;
        self.sale_reserve = sale_reserve;
        self.real_collateral_reserve = real_collateral_reserve;
        self.open = !auto_closed;

        Ok(BuyOutcome {
            gross_collateral_in,
            effective_collateral_in,
            pool_fee,
            protocol_fee,
            tokens_out,
            auto_closed,
        })
    }

    /// Sells tokens back for collateral. Fees are charged in the input token,
    /// using the same combined-rounding and deterministic split as [`Sale::buy`].
    pub fn sell(
        &mut self,
        gross_tokens_in: u128,
        min_collateral_out: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
    ) -> Result<SellOutcome, TradeError> {
        if !self.open {
            return Err(TradeError::SaleClosed);
        }
        if gross_tokens_in == 0 {
            return Err(TradeError::ZeroInput);
        }

        let (effective_tokens_in, pool_fee, protocol_fee) =
            split_input(gross_tokens_in, pool_fee_bps, protocol_fee_bps)?;
        let old_product = self
            .virtual_token_reserve
            .checked_mul(self.virtual_collateral_reserve)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let collateral_out = sell_collateral_out_floor(
            self.virtual_token_reserve,
            self.virtual_collateral_reserve,
            old_product,
            effective_tokens_in,
        )?;
        if collateral_out < min_collateral_out {
            return Err(TradeError::SlippageExceeded);
        }
        if collateral_out > self.real_collateral_reserve {
            return Err(TradeError::ExceedsRealCollateralReserve);
        }

        let priced_token_reserve = self
            .virtual_token_reserve
            .checked_add(effective_tokens_in)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let virtual_token_reserve = priced_token_reserve
            .checked_add(pool_fee)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let virtual_collateral_reserve = self
            .virtual_collateral_reserve
            .checked_sub(collateral_out)
            .ok_or(TradeError::ArithmeticOverflow)?;
        virtual_token_reserve
            .checked_mul(virtual_collateral_reserve)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let tokens_retained = effective_tokens_in
            .checked_add(pool_fee)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let sale_reserve = self
            .sale_reserve
            .checked_add(tokens_retained)
            .ok_or(TradeError::ArithmeticOverflow)?;
        let real_collateral_reserve = self
            .real_collateral_reserve
            .checked_sub(collateral_out)
            .ok_or(TradeError::ArithmeticOverflow)?;

        self.virtual_token_reserve = virtual_token_reserve;
        self.virtual_collateral_reserve = virtual_collateral_reserve;
        self.sale_reserve = sale_reserve;
        self.real_collateral_reserve = real_collateral_reserve;

        Ok(SellOutcome {
            gross_tokens_in,
            effective_tokens_in,
            pool_fee,
            protocol_fee,
            collateral_out,
        })
    }

    /// Ends an open sale without moving either reserve. Authorization and the
    /// later creator withdrawal are chain-level concerns owned by `curve-core`.
    pub fn close(&mut self) -> Result<(), CloseError> {
        if !self.open {
            return Err(CloseError::SaleClosed);
        }
        self.open = false;
        Ok(())
    }
}

fn split_input(
    gross_input: u128,
    pool_fee_bps: u16,
    protocol_fee_bps: u16,
) -> Result<(u128, u128, u128), TradeError> {
    let total_fee_bps = pool_fee_bps
        .checked_add(protocol_fee_bps)
        .ok_or(TradeError::InvalidFeeRates)?;
    if total_fee_bps > BASIS_POINTS_DENOMINATOR {
        return Err(TradeError::InvalidFeeRates);
    }
    if total_fee_bps == 0 {
        return Ok((gross_input, 0, 0));
    }

    let combined_fee = mul_div_ceil(
        gross_input,
        u128::from(total_fee_bps),
        u128::from(BASIS_POINTS_DENOMINATOR),
    )?;
    let effective_input = gross_input
        .checked_sub(combined_fee)
        .ok_or(TradeError::ArithmeticOverflow)?;
    if effective_input == 0 {
        return Err(TradeError::InputConsumedByFees);
    }
    let protocol_fee = mul_div_floor(
        combined_fee,
        u128::from(protocol_fee_bps),
        u128::from(total_fee_bps),
    )?;
    let pool_fee = combined_fee
        .checked_sub(protocol_fee)
        .ok_or(TradeError::ArithmeticOverflow)?;
    Ok((effective_input, pool_fee, protocol_fee))
}

fn mul_div_floor(value: u128, multiplier: u128, denominator: u128) -> Result<u128, TradeError> {
    let quotient = value
        .checked_div(denominator)
        .ok_or(TradeError::ArithmeticOverflow)?;
    let remainder = value
        .checked_rem(denominator)
        .ok_or(TradeError::ArithmeticOverflow)?;
    let whole = quotient
        .checked_mul(multiplier)
        .ok_or(TradeError::ArithmeticOverflow)?;
    let fraction = remainder
        .checked_mul(multiplier)
        .ok_or(TradeError::ArithmeticOverflow)?
        .checked_div(denominator)
        .ok_or(TradeError::ArithmeticOverflow)?;
    whole
        .checked_add(fraction)
        .ok_or(TradeError::ArithmeticOverflow)
}

fn mul_div_ceil(value: u128, multiplier: u128, denominator: u128) -> Result<u128, TradeError> {
    let floor = mul_div_floor(value, multiplier, denominator)?;
    let remainder = value
        .checked_rem(denominator)
        .ok_or(TradeError::ArithmeticOverflow)?;
    let fractional_numerator = remainder
        .checked_mul(multiplier)
        .ok_or(TradeError::ArithmeticOverflow)?;
    if fractional_numerator
        .checked_rem(denominator)
        .ok_or(TradeError::ArithmeticOverflow)?
        == 0
    {
        Ok(floor)
    } else {
        floor.checked_add(1).ok_or(TradeError::ArithmeticOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_fixes_k_from_the_virtual_reserves_and_opens_the_sale() {
        let sale = Sale::create(800, 200, 1000, 100).expect("valid sale");
        assert_eq!(sale.sale_reserve, 800);
        assert_eq!(sale.dex_seed_reserve, 200);
        assert_eq!(sale.virtual_token_reserve, 1000);
        assert_eq!(sale.virtual_collateral_reserve, 100);
        assert_eq!(sale.k, 100_000);
        assert_eq!(sale.real_collateral_reserve, 0);
        assert!(sale.open);
    }

    #[test]
    fn create_requires_the_virtual_token_reserve_to_exceed_the_sale_reserve() {
        // F2. At Vt == D the final buy would need Vc to reach infinity.
        assert_eq!(
            Sale::create(1000, 0, 1000, 100),
            Err(CreateError::VirtualTokenReserveNotAboveSaleReserve)
        );
        assert_eq!(
            Sale::create(1001, 0, 1000, 100),
            Err(CreateError::VirtualTokenReserveNotAboveSaleReserve)
        );
    }

    #[test]
    fn create_bounds_both_virtual_reserves_below_two_to_the_64() {
        // The overflow argument in docs/adr/0004 rests on this bound.
        assert_eq!(
            Sale::create(800, 0, 1 << 64, 100),
            Err(CreateError::VirtualReserveAboveBound)
        );
        assert_eq!(
            Sale::create(800, 0, 1000, 1 << 64),
            Err(CreateError::VirtualReserveAboveBound)
        );
        assert!(Sale::create(800, 0, (1 << 64) - 1, (1 << 64) - 1).is_ok());
    }

    #[test]
    fn create_requires_a_virtual_collateral_reserve_to_price_the_curve() {
        // Vc = 0 fixes k = 0, and k = 0 quotes the whole virtual token
        // reserve for any collateral at all.
        assert_eq!(
            Sale::create(800, 200, 1000, 0),
            Err(CreateError::VirtualCollateralReserveZero)
        );
    }

    #[test]
    fn create_requires_a_sale_reserve_to_dispense() {
        assert_eq!(
            Sale::create(0, 200, 1000, 100),
            Err(CreateError::SaleReserveZero)
        );
    }

    #[test]
    fn a_sale_survives_a_borsh_round_trip() {
        let sale = Sale::create(800, 200, 1000, 100).expect("valid sale");
        let bytes = borsh::to_vec(&sale).expect("sale serialises");
        assert_eq!(Sale::try_from_slice(&bytes).expect("bytes parse"), sale);
    }
}
