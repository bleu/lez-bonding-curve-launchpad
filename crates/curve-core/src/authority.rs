//! Bearer authority shared by namespace, creator, and direct pool-owner roles.
use lee_core::{
    account::{AccountId, AccountWithMetadata},
    program::ProgramId,
};
use token_core::TokenHolding;

/// Image ID of the token guest at the workspace-pinned LEZ revision.
pub const TOKEN_PROGRAM_ID: ProgramId = [
    2282739141, 348907455, 1046946228, 3735699860, 585462133, 3426087150, 772528164, 2090518099,
];

/// Returns the stable identity of an authorized, indivisible NFT master.
/// Printed copies, empty former holders, and unrelated program data grant no rights.
pub fn identity(holder: &AccountWithMetadata) -> AccountId {
    assert!(holder.is_authorized, "Authority authorization is missing");
    assert_eq!(
        holder.account.program_owner, TOKEN_PROGRAM_ID,
        "Authority must be owned by the trusted token program"
    );
    match TokenHolding::try_from(&holder.account.data).expect("Invalid authority token holding") {
        TokenHolding::NftMaster {
            definition_id,
            print_balance: 1,
        } => definition_id,
        _ => panic!("Authority requires an NFT master with one remaining unit"),
    }
}

/// Internal program custody uses a specific PDA; all human roles use NFTs.
pub fn pool_owner(
    holder: &AccountWithMetadata,
    program: Option<(ProgramId, [u8; 32])>,
) -> AccountId {
    match program {
        None => identity(holder),
        Some((program_id, seed)) => {
            assert!(holder.is_authorized, "Owner authorization is missing");
            assert_eq!(
                holder.account_id,
                AccountId::for_public_pda(&program_id, &lee_core::program::PdaSeed::new(seed)),
                "Program owner must be its derived PDA"
            );
            assert_eq!(
                holder.account.program_owner, program_id,
                "Program authority belongs to another program"
            );
            holder.account_id
        }
    }
}
