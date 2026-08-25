//! SPEL interface declaration for the hand-written factory guest.

#[lez_program(instruction = "factory_core::Instruction")]
pub mod factory {
    #[instruction]
    pub fn create_factory_pool(
        #[account(init, pda = [literal("factory"), arg("launch_salt")])] factory: AccountWithMetadata,
        #[account(init, pda = [literal("definition"), arg("launch_salt")])] token_definition: AccountWithMetadata,
        #[account(init)] mint: AccountWithMetadata,
        #[account(init)] metadata: AccountWithMetadata,
        #[account(init)] creator_escrow: AccountWithMetadata,
        #[account(signer)] creator: AccountWithMetadata,
        #[account(init)] creator_token_ata: AccountWithMetadata,
        collateral_definition: AccountWithMetadata,
        #[account(init)] factory_token_ata: AccountWithMetadata,
        #[account(init)] factory_collateral_ata: AccountWithMetadata,
        #[account(init)] pool: AccountWithMetadata,
        #[account(init)] pool_token_ata: AccountWithMetadata,
        #[account(init)] pool_collateral_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
        launch_salt: [u8; 32],
        name: String,
        uri: String,
        sale_reserve: u128,
        dex_seed_reserve: u128,
        creator_allocation: u128,
        virtual_token_reserve: u128,
        virtual_collateral_reserve: u128,
        end_timestamp: Option<u64>,
        curve_program_id: ProgramId,
    ) {}

    #[instruction]
    pub fn close_factory_pool(
        factory: AccountWithMetadata,
        #[account(mut)] pool: AccountWithMetadata,
        #[account(signer)] creator: AccountWithMetadata,
        clock: AccountWithMetadata,
    ) {}

    #[instruction]
    pub fn claim_creator_allocation(
        #[account(mut)] factory: AccountWithMetadata,
        pool: AccountWithMetadata,
        #[account(mut)] creator_escrow: AccountWithMetadata,
        #[account(signer)] creator: AccountWithMetadata,
        token_definition: AccountWithMetadata,
        #[account(mut)] creator_token_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
    ) {}

    #[instruction]
    pub fn withdraw_factory_proceeds(
        #[account(mut)] factory: AccountWithMetadata,
        #[account(mut)] pool: AccountWithMetadata,
        #[account(signer)] creator: AccountWithMetadata,
        token_definition: AccountWithMetadata,
        collateral_definition: AccountWithMetadata,
        #[account(mut)] factory_token_ata: AccountWithMetadata,
        #[account(mut)] factory_collateral_ata: AccountWithMetadata,
        #[account(mut)] pool_token_ata: AccountWithMetadata,
        #[account(mut)] pool_collateral_ata: AccountWithMetadata,
        #[account(mut)] creator_token_ata: AccountWithMetadata,
        #[account(mut)] creator_collateral_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
    ) {}
}
