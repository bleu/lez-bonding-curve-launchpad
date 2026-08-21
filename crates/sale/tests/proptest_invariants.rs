use proptest::prelude::*;
use sale::{CloseError, Sale, TradeError};

#[derive(Clone, Debug)]
enum Action {
    Buy {
        gross_collateral_in: u128,
        min_tokens_out: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
    },
    Sell {
        gross_tokens_in: u128,
        min_collateral_out: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
    },
    Close,
}

fn checked_add(left: u128, right: u128) -> u128 {
    left.checked_add(right)
        .expect("property oracle addition fits")
}

fn checked_sub(left: u128, right: u128) -> u128 {
    left.checked_sub(right)
        .expect("property oracle subtraction fits")
}

fn checked_mul(left: u128, right: u128) -> u128 {
    left.checked_mul(right)
        .expect("property oracle multiplication fits")
}

fn checked_rem(left: u128, right: u128) -> u128 {
    left.checked_rem(right)
        .expect("property oracle divisor is nonzero")
}

fn checked_sum3(first: u128, second: u128, third: u128) -> u128 {
    checked_add(checked_add(first, second), third)
}

fn amount_strategy() -> impl Strategy<Value = u128> {
    prop_oneof![
        8 => 0_u128..10_000,
        2 => prop::sample::select(vec![
            0,
            1,
            (1_u128 << 64) - 1,
            1_u128 << 64,
            u128::MAX - 1,
            u128::MAX,
        ]),
    ]
}

fn fee_strategy() -> impl Strategy<Value = (u16, u16)> {
    prop_oneof![
        8 => (0_u16..=1_000, 0_u16..=1_000),
        2 => prop::sample::select(vec![
            (0, 0),
            (10_000, 0),
            (0, 10_000),
            (10_000, 1),
            (u16::MAX, u16::MAX),
        ]),
    ]
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        9 => (amount_strategy(), amount_strategy(), fee_strategy()).prop_map(
            |(gross_collateral_in, min_tokens_out, (pool_fee_bps, protocol_fee_bps))| {
                Action::Buy {
                    gross_collateral_in,
                    min_tokens_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                }
            },
        ),
        9 => (amount_strategy(), amount_strategy(), fee_strategy()).prop_map(
            |(gross_tokens_in, min_collateral_out, (pool_fee_bps, protocol_fee_bps))| {
                Action::Sell {
                    gross_tokens_in,
                    min_collateral_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                }
            },
        ),
        1 => Just(Action::Close),
    ]
}

fn sale_strategy() -> impl Strategy<Value = Sale> {
    prop_oneof![
        9 => (2_u128..1_000_000, 1_u128..1_000_000, 0_u128..1_000_000, any::<u64>())
            .prop_map(|(virtual_token_reserve, virtual_collateral_reserve, dex_seed_reserve, seed)| {
                let sale_reserve = checked_add(
                    1,
                    checked_rem(
                        u128::from(seed),
                        checked_sub(virtual_token_reserve, 1),
                    ),
                );
                Sale::create(
                    sale_reserve,
                    dex_seed_reserve,
                    virtual_token_reserve,
                    virtual_collateral_reserve,
                ).expect("generated sale is valid")
            }),
        1 => prop::sample::select(vec![
            Sale::create(1, u128::MAX, 2, (1_u128 << 64) - 1).expect("boundary sale"),
            Sale::create(
                (1_u128 << 64) - 2,
                u128::MAX,
                (1_u128 << 64) - 1,
                (1_u128 << 64) - 1,
            ).expect("maximum-reserve sale"),
        ]),
    ]
}

#[test]
fn manual_close_is_atomic_and_permanent() {
    let mut sale = Sale::create(800, 200, 1_000, 100).expect("valid sale");
    let reserves_before_close = sale.clone();

    sale.close().expect("open sale closes");

    assert!(!sale.open);
    assert_eq!(
        Sale {
            open: true,
            ..sale.clone()
        },
        reserves_before_close,
    );
    let closed = sale.clone();
    assert_eq!(sale.close(), Err(CloseError::SaleClosed));
    assert_eq!(sale, closed);
    assert_eq!(sale.buy(10, 0, 0, 0), Err(TradeError::SaleClosed));
    assert_eq!(sale, closed);
    assert_eq!(sale.sell(10, 0, 0, 0), Err(TradeError::SaleClosed));
    assert_eq!(sale, closed);
}

#[test]
fn a_buy_that_exhausts_the_sale_reserve_closes_atomically() {
    let mut sale = Sale::create(200, 50, 1_000, 100).expect("valid sale");

    let outcome = sale.buy(25, 200, 0, 0).expect("final buy succeeds");

    assert_eq!(outcome.tokens_out, 200);
    assert!(outcome.auto_closed);
    assert_eq!(sale.sale_reserve, 0);
    assert!(!sale.open);
    let closed = sale.clone();
    assert_eq!(sale.buy(1, 0, 0, 0), Err(TradeError::SaleClosed));
    assert_eq!(sale, closed);
}

#[test]
fn rejected_trades_leave_the_complete_sale_unchanged() {
    let mut sale = Sale::create(800, 200, 1_000, 100).expect("valid sale");

    for result in [
        sale.buy(0, 0, 0, 0),
        sale.buy(10, 0, 10_000, 1),
        sale.buy(10, u128::MAX, 0, 0),
        sale.buy(u128::MAX, 0, 0, 0),
    ] {
        assert!(result.is_err());
        assert_eq!(
            sale,
            Sale::create(800, 200, 1_000, 100).expect("valid sale")
        );
    }

    for result in [
        sale.sell(0, 0, 0, 0),
        sale.sell(10, 0, 10_000, 1),
        sale.sell(10, u128::MAX, 0, 0),
        sale.sell(u128::MAX, 0, 0, 0),
    ] {
        assert!(result.is_err());
        assert_eq!(
            sale,
            Sale::create(800, 200, 1_000, 100).expect("valid sale")
        );
    }
}

#[test]
fn combined_fee_rounds_once_and_splits_deterministically() {
    let mut sale = Sale::create(800, 200, 1_000, 100).expect("valid sale");

    let outcome = sale.buy(101, 0, 100, 100).expect("buy succeeds");

    // ceil(101 * 200 / 10_000) = 3. The protocol receives its proportional
    // floor share; the sale retains the remainder, so the combined fee is exact.
    assert_eq!(outcome.effective_collateral_in, 98);
    assert_eq!(outcome.protocol_fee, 1);
    assert_eq!(outcome.pool_fee, 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn a_successful_buy_preserves_solvency_conservation_and_k(
        virtual_token_reserve in 2_u128..1_000_000,
        virtual_collateral_reserve in 1_u128..1_000_000,
        gross_collateral_in in 2_u128..1_000_000,
        pool_fee_bps in 0_u16..=1_000,
        protocol_fee_bps in 0_u16..=1_000,
    ) {
        let mut sale = Sale::create(
            checked_sub(virtual_token_reserve, 1),
            virtual_token_reserve / 5,
            virtual_token_reserve,
            virtual_collateral_reserve,
        ).expect("generated sale is valid");
        let before = sale.clone();
        let old_product = checked_mul(
            before.virtual_token_reserve,
            before.virtual_collateral_reserve,
        );

        let outcome = sale.buy(
            gross_collateral_in,
            0,
            pool_fee_bps,
            protocol_fee_bps,
        ).expect("generated buy is admissible");

        prop_assert!(outcome.tokens_out > 0);
        prop_assert_eq!(
            outcome.gross_collateral_in,
            checked_sum3(
                outcome.effective_collateral_in,
                outcome.pool_fee,
                outcome.protocol_fee,
            ),
        );
        prop_assert_eq!(
            sale.real_collateral_reserve,
            checked_sub(
                checked_add(
                    before.real_collateral_reserve,
                    outcome.gross_collateral_in,
                ),
                outcome.protocol_fee,
            ),
        );
        prop_assert_eq!(
            checked_add(sale.sale_reserve, outcome.tokens_out),
            before.sale_reserve,
        );
        prop_assert_eq!(sale.dex_seed_reserve, before.dex_seed_reserve);
        prop_assert_eq!(sale.k, before.k);
        let new_product = checked_mul(
            sale.virtual_token_reserve,
            sale.virtual_collateral_reserve,
        );
        prop_assert!(new_product >= old_product);
        prop_assert!(new_product >= sale.k);
    }

    #[test]
    fn a_successful_sell_is_backed_and_preserves_conservation_and_k(
        gross_tokens_in in 10_u128..=200,
        pool_fee_bps in 0_u16..=1_000,
        protocol_fee_bps in 0_u16..=1_000,
    ) {
        let mut sale = Sale::create(800, 200, 1_000, 100).expect("valid sale");
        let bought = sale.buy(25, 0, 0, 0).expect("fixture buy succeeds");
        prop_assert_eq!(bought.tokens_out, 200);
        let before = sale.clone();
        let old_product = checked_mul(
            before.virtual_token_reserve,
            before.virtual_collateral_reserve,
        );

        let outcome = sale.sell(
            gross_tokens_in,
            0,
            pool_fee_bps,
            protocol_fee_bps,
        ).expect("generated sell is admissible");

        prop_assert_eq!(
            outcome.gross_tokens_in,
            checked_sum3(
                outcome.effective_tokens_in,
                outcome.pool_fee,
                outcome.protocol_fee,
            ),
        );
        prop_assert!(outcome.collateral_out <= before.real_collateral_reserve);
        prop_assert_eq!(
            checked_add(sale.real_collateral_reserve, outcome.collateral_out),
            before.real_collateral_reserve,
        );
        prop_assert_eq!(
            sale.sale_reserve,
            checked_sub(
                checked_add(before.sale_reserve, outcome.gross_tokens_in),
                outcome.protocol_fee,
            ),
        );
        prop_assert_eq!(sale.dex_seed_reserve, before.dex_seed_reserve);
        prop_assert_eq!(sale.k, before.k);
        let new_product = checked_mul(
            sale.virtual_token_reserve,
            sale.virtual_collateral_reserve,
        );
        prop_assert!(new_product >= old_product);
        prop_assert!(new_product >= sale.k);
    }

    #[test]
    fn randomized_interleavings_preserve_solvency_and_are_atomic(
        mut sale in sale_strategy(),
        actions in prop::collection::vec(action_strategy(), 1..=128),
    ) {
        let initial_k = sale.k;
        let initial_sale_reserve = sale.sale_reserve;
        let initial_dex_seed_reserve = sale.dex_seed_reserve;
        let mut collateral_inflows = 0_u128;
        let mut collateral_outflows = 0_u128;
        let mut tokens_dispensed = 0_u128;
        let mut tokens_returned = 0_u128;

        for action in actions {
            let before = sale.clone();
            let old_product = checked_mul(
                before.virtual_token_reserve,
                before.virtual_collateral_reserve,
            );
            let mut expect_strict_product_increase = false;

            match action {
                Action::Buy {
                    gross_collateral_in,
                    min_tokens_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                } => match sale.buy(
                    gross_collateral_in,
                    min_tokens_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                ) {
                    Ok(outcome) => {
                        prop_assert!(gross_collateral_in > 0);
                        prop_assert_eq!(
                            outcome.gross_collateral_in,
                            checked_sum3(
                                outcome.effective_collateral_in,
                                outcome.pool_fee,
                                outcome.protocol_fee,
                            ),
                        );
                        let retained = checked_add(outcome.effective_collateral_in, outcome.pool_fee);
                        expect_strict_product_increase = outcome.pool_fee > 0;
                        collateral_inflows = checked_add(collateral_inflows, retained);
                        tokens_dispensed = checked_add(tokens_dispensed, outcome.tokens_out);
                        prop_assert_eq!(
                            sale.real_collateral_reserve,
                            checked_add(before.real_collateral_reserve, retained),
                        );
                        prop_assert_eq!(
                            checked_add(sale.sale_reserve, outcome.tokens_out),
                            before.sale_reserve,
                        );
                        if outcome.auto_closed {
                            prop_assert_eq!(sale.sale_reserve, 0);
                            prop_assert!(!sale.open);
                        }
                    }
                    Err(_) => prop_assert_eq!(&sale, &before),
                },
                Action::Sell {
                    gross_tokens_in,
                    min_collateral_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                } => match sale.sell(
                    gross_tokens_in,
                    min_collateral_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                ) {
                    Ok(outcome) => {
                        prop_assert!(gross_tokens_in > 0);
                        prop_assert_eq!(
                            outcome.gross_tokens_in,
                            checked_sum3(
                                outcome.effective_tokens_in,
                                outcome.pool_fee,
                                outcome.protocol_fee,
                            ),
                        );
                        prop_assert!(outcome.collateral_out <= before.real_collateral_reserve);
                        let retained = checked_add(outcome.effective_tokens_in, outcome.pool_fee);
                        expect_strict_product_increase = outcome.pool_fee > 0;
                        tokens_returned = checked_add(tokens_returned, retained);
                        collateral_outflows = checked_add(collateral_outflows, outcome.collateral_out);
                        prop_assert_eq!(
                            checked_add(sale.real_collateral_reserve, outcome.collateral_out),
                            before.real_collateral_reserve,
                        );
                        prop_assert_eq!(
                            sale.sale_reserve,
                            checked_add(before.sale_reserve, retained),
                        );
                    }
                    Err(_) => prop_assert_eq!(&sale, &before),
                },
                Action::Close => match sale.close() {
                    Ok(()) => {
                        prop_assert!(before.open);
                        prop_assert!(!sale.open);
                        prop_assert_eq!(
                            Sale { open: true, ..sale.clone() },
                            before,
                        );
                    }
                    Err(_) => prop_assert_eq!(&sale, &before),
                },
            }

            let new_product = checked_mul(
                sale.virtual_token_reserve,
                sale.virtual_collateral_reserve,
            );
            prop_assert!(new_product >= old_product);
            if expect_strict_product_increase {
                prop_assert!(new_product > old_product);
            }
            prop_assert!(new_product >= initial_k);
            prop_assert_eq!(sale.k, initial_k);
            prop_assert_eq!(sale.dex_seed_reserve, initial_dex_seed_reserve);
            prop_assert_eq!(
                sale.real_collateral_reserve,
                checked_sub(collateral_inflows, collateral_outflows),
            );
            prop_assert_eq!(
                sale.sale_reserve,
                checked_sub(
                    checked_add(initial_sale_reserve, tokens_returned),
                    tokens_dispensed,
                ),
            );
        }
    }
}
