//! Resumable factory workflows. Every stage is confirmed before the next is built.
use super::*;
use factory_core::{CreationStage, SettlementStage};

#[derive(Debug, Default)]
pub struct LifecycleReceipt {
    pub transaction_hashes: Vec<HashType>,
    pub authority_holders: Vec<AccountId>,
}

/// A factory workflow bound to one namespace and NFT authority source.
pub struct FactorySession<'a> {
    wallet: &'a mut WalletCore,
    factory: &'a Program,
    curve: &'a Program,
    router: Option<&'a Program>,
    namespace: AccountId,
    authority: &'a str,
}
impl<'a> FactorySession<'a> {
    pub fn new(
        wallet: &'a mut WalletCore,
        factory: &'a Program,
        curve: &'a Program,
        router: Option<&'a Program>,
        namespace: AccountId,
        authority: &'a str,
    ) -> Self {
        Self {
            wallet,
            factory,
            curve,
            router,
            namespace,
            authority,
        }
    }
    async fn state(&self, salt: [u8; 32]) -> Result<Option<FactoryState>> {
        let account = self
            .wallet
            .get_account_public(compute_factory_pda(self.namespace, self.factory.id(), salt))
            .await?;
        if account == lee_core::account::Account::default() {
            return Ok(None);
        }
        anyhow::ensure!(
            account.program_owner == self.factory.id(),
            "factory account belongs to another program"
        );
        Ok(Some(
            FactoryState::try_from(&account.data).context("decoding resumable factory state")?,
        ))
    }
    async fn submit(
        &mut self,
        build: impl FnOnce(AccountId) -> Result<PublicInvocation<FactoryInstruction>>,
        receipt: &mut LifecycleReceipt,
    ) -> Result<()> {
        let source = parse_account_id(self.authority)?;
        if self.authority.starts_with("Private/") {
            let result = submit_private_authority(
                self.wallet,
                self.router
                    .context("private authority router is required")?,
                self.factory,
                vec![self.curve.clone()],
                source,
                build,
            )
            .await?;
            receipt.transaction_hashes.push(result.transaction_hash);
            receipt
                .authority_holders
                .push(result.transient_public_account);
        } else {
            receipt
                .transaction_hashes
                .push(submit_public_invocation(self.wallet, self.factory, build(source)?).await?);
            receipt.authority_holders.push(source);
        }
        Ok(())
    }
    /// Resume from on-chain state; completed creation is an idempotent no-op.
    pub async fn create_sale(&mut self, request: CreateSaleRequest) -> Result<LifecycleReceipt> {
        let mut receipt = LifecycleReceipt::default();
        for _ in 0..4 {
            let state = self.state(request.launch_salt).await?;
            let namespace = self.namespace;
            let factory = self.factory.id();
            let curve = self.curve.id();
            if next_creation_invocation(
                namespace,
                factory,
                curve,
                parse_account_id(self.authority)?,
                request.clone(),
                state.as_ref(),
            )?
            .is_none()
            {
                return Ok(receipt);
            }
            self.submit(
                |holder| {
                    next_creation_invocation(
                        namespace,
                        factory,
                        curve,
                        holder,
                        request.clone(),
                        state.as_ref(),
                    )?
                    .context("creation already completed")
                },
                &mut receipt,
            )
            .await?;
        }
        anyhow::ensure!(
            self.state(request.launch_salt)
                .await?
                .is_some_and(|s| s.creation_stage == CreationStage::Active),
            "creation did not complete; retry with the same launch salt"
        );
        Ok(receipt)
    }
    /// Resume the exact withdrawal/burn/payout sequence without paying a stage twice.
    pub async fn withdraw(
        &mut self,
        salt: [u8; 32],
        collateral: AccountId,
    ) -> Result<LifecycleReceipt> {
        let mut receipt = LifecycleReceipt::default();
        for _ in 0..5 {
            let state = self
                .state(salt)
                .await?
                .context("factory launch does not exist")?;
            anyhow::ensure!(
                state.collateral_definition_id == collateral
                    && state.curve_program_id == self.curve.id(),
                "launch does not match the selected collateral or curve"
            );
            if state.settlement_stage == SettlementStage::Complete {
                return Ok(receipt);
            }
            let namespace = self.namespace;
            let factory = self.factory.id();
            let curve = self.curve.id();
            // A closed pool cannot accrue new fees. Collect publicly before the private
            // stage to keep its call graph within the pinned privacy circuit budget.
            if self.authority.starts_with("Private/")
                && state.settlement_stage == SettlementStage::Ready
            {
                receipt.transaction_hashes.extend(
                    collect_pool_fees(
                        self.wallet,
                        namespace,
                        self.curve,
                        &[(state.pool_id, collateral)],
                    )
                    .await?,
                );
            }
            let treasury =
                prepare_fee_collection(self.wallet, namespace, curve, collateral).await?;
            self.submit(
                |holder| {
                    Ok(build_withdraw_factory_proceeds_invocation(
                        namespace, factory, curve, holder, salt, collateral, treasury,
                    ))
                },
                &mut receipt,
            )
            .await?;
        }
        anyhow::ensure!(
            self.state(salt)
                .await?
                .is_some_and(|s| s.settlement_stage == SettlementStage::Complete),
            "settlement did not complete; retry with the same launch salt"
        );
        Ok(receipt)
    }
}

/// Chooses the next wire call from persisted state; never resubmits the mint stage.
pub fn next_creation_invocation(
    namespace: AccountId,
    factory: ProgramId,
    curve: ProgramId,
    holder: AccountId,
    request: CreateSaleRequest,
    state: Option<&FactoryState>,
) -> Result<Option<PublicInvocation<FactoryInstruction>>> {
    if let Some(state) = state {
        anyhow::ensure!(
            state.namespace == namespace
                && state.launch_salt == request.launch_salt
                && state.curve_program_id == curve
                && state.collateral_definition_id == request.collateral_definition
                && state.sale_reserve == request.sale_reserve
                && state.dex_seed_reserve == request.dex_seed_reserve
                && state.creator_allocation == request.creator_allocation
                && state.virtual_token_reserve == request.virtual_token_reserve
                && state.virtual_collateral_reserve == request.virtual_collateral_reserve
                && state.end_timestamp == request.end_timestamp,
            "existing launch parameters differ; resume with the original parameters"
        );
        if state.creation_stage == CreationStage::Active {
            return Ok(None);
        }
    }
    let mut call = build_create_sale_invocation(namespace, factory, curve, holder, request)?;
    if state.is_some() {
        call.instruction = FactoryInstruction::ContinueCreation;
    }
    Ok(Some(call))
}
