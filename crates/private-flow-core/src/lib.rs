//! Wire instruction for the private-buy router guest.

use lee_core::{account::AccountId, program::ProgramId};
use serde::{Deserialize, Serialize};

/// Executes the RFP-015 private purchase composition in one privacy-preserving transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateBuyInstruction {
    pub curve_program_id: ProgramId,
    pub token_program_id: ProgramId,
    pub ata_program_id: ProgramId,
    pub native_transfer_program_id: ProgramId,
    pub amount_out: u128,
    pub max_collateral_in: u128,
    pub gas_reserve: u128,
    pub collateral_definition: AccountId,
}

/// One private invocation: expose the authority NFT, act, return it to its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateAuthorityInstruction {
    pub program_id: ProgramId,
    pub instruction_data: Vec<u32>,
    /// Index of the fresh public NFT holding in the action's account list.
    pub authority_index: usize,
}

/// Accounts: private NFT source, followed by the action's accounts (one fresh public holder).
/// Each chained call carries the exact state produced by its predecessor.
pub fn authority_action(
    pre_states: Vec<lee_core::account::AccountWithMetadata>,
    instruction: PrivateAuthorityInstruction,
) -> (
    Vec<lee_core::program::AccountPostState>,
    Vec<lee_core::program::ChainedCall>,
) {
    use lee_core::{
        account::{Account, Data},
        program::{AccountPostState, ChainedCall},
    };
    use token_core::{Instruction as TokenInstruction, TokenHolding};
    let (source, action_accounts) = pre_states
        .split_first()
        .expect("Authority source is required");
    let identity = curve_core::authority::identity(source);
    let transient = action_accounts
        .get(instruction.authority_index)
        .expect("Authority account index is out of bounds");
    assert!(
        transient.is_authorized,
        "Transient holder must be authorized"
    );
    assert_eq!(
        transient.account,
        Account::default(),
        "Transient holder must be fresh"
    );
    assert_ne!(
        source.account_id, transient.account_id,
        "Authority source must differ from transient holder"
    );
    let mut funded = transient.clone();
    funded.account.program_owner = curve_core::authority::TOKEN_PROGRAM_ID;
    funded.account.data = source.account.data.clone();
    let mut empty_source = source.clone();
    empty_source.account.data = Data::from(&TokenHolding::NftMaster {
        definition_id: identity,
        print_balance: 0,
    });
    let mut action_accounts = action_accounts.to_vec();
    action_accounts[instruction.authority_index] = funded.clone();
    let transfer = TokenInstruction::Transfer {
        amount_to_transfer: 1,
    };
    let calls = vec![
        ChainedCall::new(
            curve_core::authority::TOKEN_PROGRAM_ID,
            vec![source.clone(), transient.clone()],
            &transfer,
        ),
        ChainedCall {
            program_id: instruction.program_id,
            pre_states: action_accounts,
            instruction_data: instruction.instruction_data,
            pda_seeds: vec![],
        },
        ChainedCall::new(
            curve_core::authority::TOKEN_PROGRAM_ID,
            vec![funded, empty_source],
            &transfer,
        ),
    ];
    (
        pre_states
            .into_iter()
            .map(|pre| AccountPostState::new(pre.account))
            .collect(),
        calls,
    )
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use lee_core::account::{Account, AccountWithMetadata, Data};

    #[test]
    fn private_authority_action_receives_nft_and_returns_it_to_private_source() {
        let namespace = AccountId::new([31; 32]);
        let source = AccountWithMetadata {
            account_id: AccountId::new([32; 32]),
            is_authorized: true,
            account: Account {
                program_owner: curve_core::authority::TOKEN_PROGRAM_ID,
                data: Data::from(&token_core::TokenHolding::NftMaster {
                    definition_id: namespace,
                    print_balance: 1,
                }),
                ..Account::default()
            },
        };
        let config = AccountWithMetadata {
            account_id: curve_core::compute_config_pda(namespace, [7; 8]),
            account: Account::default(),
            is_authorized: false,
        };
        let transient = AccountWithMetadata {
            account_id: AccountId::new([33; 32]),
            account: Account::default(),
            is_authorized: true,
        };
        let pre = vec![source, config, transient];
        let instruction = PrivateAuthorityInstruction {
            program_id: [7; 8],
            authority_index: 1,
            instruction_data: risc0_zkvm::serde::to_vec(&curve_core::Instruction::UpdateConfig {
                namespace,
                admin: namespace,
                protocol_fee_bps: 25,
                treasury: AccountId::new([34; 32]),
            })
            .unwrap(),
        };
        let (post, calls) = authority_action(pre.clone(), instruction);
        lee_core::program::validate_execution(&pre, &post, [8; 8]).unwrap();
        assert_eq!(calls.len(), 3);
        let action: curve_core::Instruction =
            risc0_zkvm::serde::from_slice(&calls[1].instruction_data).unwrap();
        let (action_post, _) =
            curve_core::dispatch::process_instruction(calls[1].pre_states.clone(), action, [7; 8]);
        lee_core::program::validate_execution(&calls[1].pre_states, &action_post, [7; 8]).unwrap();
        assert_eq!(
            curve_core::Config::try_from(&action_post[0].account().data)
                .unwrap()
                .protocol_fee_bps,
            25
        );
        assert_eq!(calls[2].pre_states[1].account_id, pre[0].account_id);
        assert_eq!(
            token_core::TokenHolding::try_from(&calls[2].pre_states[1].account.data).unwrap(),
            token_core::TokenHolding::NftMaster {
                definition_id: namespace,
                print_balance: 0
            }
        );
    }
}
