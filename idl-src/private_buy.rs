//! SPEL interface declaration for the private-buy router guest.

#[lez_program(instruction = "private_flow_core::PrivateBuyInstruction")]
pub mod private_buy {
    #[instruction]
    pub fn execute(
        #[account(signer)] private_source: AccountWithMetadata,
        #[account(signer)] transient: AccountWithMetadata,
        collateral_definition: AccountWithMetadata,
        launch_definition: AccountWithMetadata,
        #[account(init)] transient_collateral_ata: AccountWithMetadata,
        #[account(init)] transient_token_ata: AccountWithMetadata,
        #[account(mut)] pool: AccountWithMetadata,
        config: AccountWithMetadata,
        #[account(mut)] pool_collateral_ata: AccountWithMetadata,
        #[account(mut)] pool_token_ata: AccountWithMetadata,
        #[account(mut)] treasury_collateral_ata: AccountWithMetadata,
        clock: AccountWithMetadata,
        #[account(mut)] private_destination: AccountWithMetadata,
        curve_program_id: ProgramId,
        token_program_id: ProgramId,
        ata_program_id: ProgramId,
        native_transfer_program_id: ProgramId,
        amount_out: u128,
        max_collateral_in: u128,
        gas_reserve: u128,
        collateral_definition_id: AccountId,
    ) {}
}
