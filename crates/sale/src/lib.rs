//! The sale state machine: a buy, a sell, or a close applied to the sale's own data.
//!
//! Knows about virtual reserves, the two accounting buckets, fees, slippage, and the
//! open or closed flag. Knows nothing about `AccountWithMetadata`, PDAs, or
//! `ChainedCall`, which is what lets the solvency property test run against a state
//! machine instead of hand-built account fixtures.
//!
//! The buy, sell, and close transitions arrive with GTM-510, GTM-511 and GTM-512.
//! GTM-514 targets this crate with the solvency property test.

use borsh::{BorshDeserialize, BorshSerialize};

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
