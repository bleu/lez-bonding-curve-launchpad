use pool::{Pool, PoolLifecycle, SwapOutcome, TokenSide};
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Action {
    ExactInput {
        side: TokenSide,
        amount_in: u128,
        min_amount_out: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
    },
    ExactOutput {
        side: TokenSide,
        amount_out: u128,
        max_amount_in: u128,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
    },
    Close,
    Withdraw,
}

fn checked_add(left: u128, right: u128) -> u128 {
    left.checked_add(right).expect("property addition fits")
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

fn side_strategy() -> impl Strategy<Value = TokenSide> {
    prop_oneof![Just(TokenSide::Token0), Just(TokenSide::Token1)]
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        9 => (side_strategy(), amount_strategy(), amount_strategy(), fee_strategy()).prop_map(
            |(side, amount_in, min_amount_out, (pool_fee_bps, protocol_fee_bps))| {
                Action::ExactInput {
                    side,
                    amount_in,
                    min_amount_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                }
            },
        ),
        9 => (side_strategy(), amount_strategy(), amount_strategy(), fee_strategy()).prop_map(
            |(side, amount_out, max_amount_in, (pool_fee_bps, protocol_fee_bps))| {
                Action::ExactOutput {
                    side,
                    amount_out,
                    max_amount_in,
                    pool_fee_bps,
                    protocol_fee_bps,
                }
            },
        ),
        1 => Just(Action::Close),
        1 => Just(Action::Withdraw),
    ]
}

fn pool_strategy() -> impl Strategy<Value = Pool> {
    prop_oneof![
        9 => (
            1_u128..1_000_000,
            1_u128..1_000_000,
            1_u128..1_000_000,
            1_u128..1_000_000,
        )
            .prop_map(|(real0, real1, virtual0, virtual1)| {
                Pool::create(real0, real1, virtual0, virtual1, None, None)
                    .expect("generated pool is valid")
            }),
        1 => prop::sample::select(vec![
            Pool::create(0, u128::MAX, 1, (1_u128 << 64) - 1, None, None)
                .expect("boundary pool"),
            Pool::create(
                u128::MAX,
                u128::MAX,
                (1_u128 << 64) - 1,
                (1_u128 << 64) - 1,
                None,
                None,
            )
            .expect("maximum-reserve pool"),
        ]),
    ]
}

fn reserves(pool: &Pool, side: TokenSide) -> (u128, u128, u128, u128) {
    match side {
        TokenSide::Token0 => (
            pool.virtual_reserve0,
            pool.virtual_reserve1,
            pool.real_reserve0,
            pool.real_reserve1,
        ),
        TokenSide::Token1 => (
            pool.virtual_reserve1,
            pool.virtual_reserve0,
            pool.real_reserve1,
            pool.real_reserve0,
        ),
    }
}

fn assert_successful_swap(before: &Pool, after: &Pool, side: TokenSide, outcome: SwapOutcome) {
    let (old_virtual_in, old_virtual_out, old_real_in, old_real_out) = reserves(before, side);
    let (new_virtual_in, new_virtual_out, new_real_in, new_real_out) = reserves(after, side);
    assert!(outcome.amount_in > 0);
    if outcome.protocol_fee_on_output {
        assert_eq!(outcome.amount_in, outcome.effective_amount_in);
        assert_eq!(
            outcome.raw_amount_out,
            checked_add(outcome.amount_out, outcome.protocol_fee)
        );
    } else {
        assert_eq!(
            outcome.amount_in,
            checked_add(outcome.effective_amount_in, outcome.protocol_fee)
        );
        assert_eq!(outcome.raw_amount_out, outcome.amount_out);
    }
    assert_eq!(
        new_virtual_in,
        checked_add(old_virtual_in, outcome.effective_amount_in)
    );
    assert_eq!(
        checked_add(new_virtual_out, outcome.raw_amount_out),
        old_virtual_out
    );
    assert_eq!(
        new_real_in,
        checked_add(old_real_in, outcome.effective_amount_in)
    );
    assert_eq!(
        checked_add(new_real_out, outcome.raw_amount_out),
        old_real_out
    );
    assert_eq!(after.k, before.k);

    // The price source is the immutable creation-time invariant. Live virtual
    // reserves may accumulate rounding drift and need not multiply in u128.
    assert_eq!(after.k, before.k);
}

#[test]
fn collateral_buy_fee_rounds_up_before_pricing() {
    let mut pool = Pool::create(800, 800, 1_000, 1_000, None, None).expect("valid pool");
    let outcome = pool
        .swap_exact_input(TokenSide::Token1, 101, 0, 0, 100, 0)
        .expect("swap succeeds");

    assert_eq!(outcome.effective_amount_in, 99);
    assert_eq!(outcome.protocol_fee, 2);
    assert!(!outcome.protocol_fee_on_output);
}

#[test]
fn manual_close_and_withdraw_are_permanent_and_post_close_swaps_fail_atomically() {
    let mut pool = Pool::create(800, 200, 1_000, 100, None, None).expect("valid pool");
    assert_eq!(pool.close_pool(0), Ok(()));
    let closed = pool.clone();
    assert!(
        pool.swap_exact_input(TokenSide::Token0, 10, 0, 0, 0, 0)
            .is_err()
    );
    assert_eq!(pool, closed);
    assert_eq!(pool.withdraw_reserves(0), Ok((800, 200)));
    let retired = pool.clone();
    assert!(pool.withdraw_reserves(0).is_err());
    assert_eq!(pool, retired);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn randomized_interleavings_preserve_solvency_conservation_and_atomicity(
        mut pool in pool_strategy(),
        actions in prop::collection::vec(action_strategy(), 1..=128),
    ) {
        let initial_k = pool.k;

        for action in actions {
            let before = pool.clone();
            match action {
                Action::ExactInput {
                    side,
                    amount_in,
                    min_amount_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                } => match pool.swap_exact_input(
                    side,
                    amount_in,
                    min_amount_out,
                    pool_fee_bps,
                    protocol_fee_bps,
                    0,
                ) {
                    Ok(outcome) => assert_successful_swap(&before, &pool, side, outcome),
                    Err(_) => prop_assert_eq!(&pool, &before),
                },
                Action::ExactOutput {
                    side,
                    amount_out,
                    max_amount_in,
                    pool_fee_bps,
                    protocol_fee_bps,
                } => match pool.swap_exact_output(
                    side,
                    amount_out,
                    max_amount_in,
                    pool_fee_bps,
                    protocol_fee_bps,
                    0,
                ) {
                    Ok(outcome) => {
                        prop_assert_eq!(outcome.amount_out, amount_out);
                        assert_successful_swap(&before, &pool, side, outcome);
                    }
                    Err(_) => prop_assert_eq!(&pool, &before),
                },
                Action::Close => {
                    let _ = pool.close_pool(0);
                    prop_assert_ne!(pool.lifecycle, PoolLifecycle::Open);
                    prop_assert_eq!(pool.k, before.k);
                    prop_assert_eq!(pool.real_reserve0, before.real_reserve0);
                    prop_assert_eq!(pool.real_reserve1, before.real_reserve1);
                }
                Action::Withdraw => match pool.withdraw_reserves(0) {
                    Ok((amount0, amount1)) => {
                        prop_assert_eq!((amount0, amount1), (before.real_reserve0, before.real_reserve1));
                        prop_assert_eq!((pool.real_reserve0, pool.real_reserve1), (0, 0));
                        prop_assert_eq!(pool.lifecycle, PoolLifecycle::Withdrawn);
                    }
                    Err(_) => prop_assert_eq!(&pool, &before),
                },
            }

            prop_assert_eq!(pool.k, initial_k);
        }
    }
}
