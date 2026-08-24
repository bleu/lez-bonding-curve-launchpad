//! SPEL interface declaration for the hand-written curve guest.
//!
//! This source is intentionally declarative: `methods/guest/src/bin/curve.rs`
//! owns execution and `curve-core::Instruction` owns the wire enum.  SPEL scans
//! this file to produce the public interface without changing either boundary.

#[lez_program(instruction = "curve_core::Instruction")]
pub mod curve {
    #[instruction]
    pub fn update_config(
        #[account(mut, pda = literal("config"))] config: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        admin: AccountId,
        protocol_fee_bps: u16,
        treasury: AccountId,
    ) {}

    #[instruction]
    pub fn create_pool(
        #[account(init, pda = [literal("pool"), account("token0_definition"), account("token1_definition"), arg("owner")])]
        pool: AccountWithMetadata,
        #[account(signer)] owner_authority: AccountWithMetadata,
        token0_definition: AccountWithMetadata,
        token1_definition: AccountWithMetadata,
        #[account(mut)] owner_token0_ata: AccountWithMetadata,
        #[account(mut)] owner_token1_ata: AccountWithMetadata,
        #[account(init)] pool_token0_ata: AccountWithMetadata,
        #[account(init)] pool_token1_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
        token0_amount: u128,
        token1_amount: u128,
        virtual_reserve0: u128,
        virtual_reserve1: u128,
        close_timestamp: Option<u64>,
        close_on_depletion: Option<DepletionSide>,
        owner: AccountId,
        curve_program_id: ProgramId,
    ) {}

    #[instruction]
    pub fn swap_exact_input(
        #[account(mut)] pool: AccountWithMetadata,
        config: AccountWithMetadata,
        #[account(signer)] participant: AccountWithMetadata,
        #[account(mut)] participant_token_in_ata: AccountWithMetadata,
        #[account(mut)] pool_token_in_ata: AccountWithMetadata,
        #[account(mut)] pool_token_out_ata: AccountWithMetadata,
        #[account(mut)] participant_token_out_ata: AccountWithMetadata,
        #[account(mut)] treasury_collateral_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
        amount_in: u128,
        min_amount_out: u128,
        token_in: AccountId,
    ) {}

    #[instruction]
    pub fn swap_exact_output(
        #[account(mut)] pool: AccountWithMetadata,
        config: AccountWithMetadata,
        #[account(signer)] participant: AccountWithMetadata,
        #[account(mut)] participant_collateral_ata: AccountWithMetadata,
        #[account(mut)] pool_collateral_ata: AccountWithMetadata,
        #[account(mut)] pool_token_ata: AccountWithMetadata,
        #[account(mut)] participant_token_ata: AccountWithMetadata,
        #[account(mut)] treasury_collateral_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
        amount_out: u128,
        max_amount_in: u128,
        token_in: AccountId,
    ) {}

    #[instruction]
    pub fn close_pool(
        #[account(mut)] pool: AccountWithMetadata,
        #[account(signer)] owner: AccountWithMetadata,
        clock: AccountWithMetadata,
    ) {}

    #[instruction]
    pub fn withdraw_reserves(
        #[account(mut)] pool: AccountWithMetadata,
        #[account(signer)] owner: AccountWithMetadata,
        token0_definition: AccountWithMetadata,
        token1_definition: AccountWithMetadata,
        #[account(mut)] pool_token0_ata: AccountWithMetadata,
        #[account(mut)] pool_token1_ata: AccountWithMetadata,
        #[account(mut)] owner_token0_ata: AccountWithMetadata,
        #[account(mut)] owner_token1_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
    ) {}
}
