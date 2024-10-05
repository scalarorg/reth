//! Ethereum block executor.

use super::{
    eth_evm_executor::ParallelEthExecuteOutput, ParallelEthEvmExecutor, ParallelEvmContext,
};
use crate::dao_fork::{DAO_HARDFORK_BENEFICIARY, DAO_HARDKFORK_ACCOUNTS};
use alloc::sync::Arc;
use alloy_primitives::U256;
use core::fmt::Display;
use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_evm::{
    execute::{
        BlockExecutionError, BlockExecutionInput, BlockExecutionOutput, BlockValidationError,
        Executor, ProviderError,
    },
    ConfigureEvm,
};
use reth_primitives::{BlockWithSenders, EthereumHardfork, Header, Receipt};
use reth_revm::{
    db::{states::bundle_state::BundleRetention, State},
    state_change::post_block_balance_increments,
};
use revm_primitives::{db::Database, BlockEnv, CfgEnvWithHandlerCfg, EnvWithHandlerCfg};
/// A basic Ethereum block executor.
///
/// Expected usage:
/// - Create a new instance of the executor.
/// - Execute the block.
#[derive(Debug)]
pub struct ParallelEthBlockExecutor<EvmConfig, DB> {
    /// Chain specific evm config that's used to execute a block.
    executor: ParallelEthEvmExecutor<EvmConfig>,
    /// The state to use for execution
    pub(super) state: State<DB>,
}

impl<EvmConfig, DB> ParallelEthBlockExecutor<EvmConfig, DB> {
    /// Creates a new Ethereum block executor.
    pub fn new(chain_spec: Arc<ChainSpec>, evm_config: EvmConfig, state: State<DB>) -> Self {
        Self { executor: ParallelEthEvmExecutor::new(chain_spec, evm_config), state }
    }

    #[inline]
    pub(super) fn chain_spec(&self) -> &ChainSpec {
        &self.executor.chain_spec
    }

    /// Returns mutable reference to the state that wraps the underlying database.
    #[allow(unused)]
    pub(super) fn state_mut(&mut self) -> &mut State<DB> {
        &mut self.state
    }
}

impl<EvmConfig, DB> ParallelEthBlockExecutor<EvmConfig, DB>
where
    EvmConfig:
        for<'a> ConfigureEvm<Header = Header, DefaultExternalContext<'a> = ParallelEvmContext>,
    DB: Database<Error: Into<ProviderError> + Display>,
{
    /// Configures a new evm configuration and block environment for the given block.
    ///
    /// # Caution
    ///
    /// This does not initialize the tx environment.
    fn evm_env_for_block(&self, header: &Header, total_difficulty: U256) -> EnvWithHandlerCfg {
        let mut cfg = CfgEnvWithHandlerCfg::new(Default::default(), Default::default());
        let mut block_env = BlockEnv::default();
        self.executor.evm_config.fill_cfg_and_block_env(
            &mut cfg,
            &mut block_env,
            header,
            total_difficulty,
        );

        EnvWithHandlerCfg::new_with_cfg_env(cfg, block_env, Default::default())
    }

    /// Execute a single block and apply the state changes to the internal state.
    ///
    /// Returns the receipts of the transactions in the block, the total gas used and the list of
    /// EIP-7685 [requests](Request).
    ///
    /// Returns an error if execution fails.
    pub(super) fn execute_without_verification(
        &mut self,
        block: &BlockWithSenders,
        total_difficulty: U256,
    ) -> Result<ParallelEthExecuteOutput, BlockExecutionError> {
        // 1. prepare state on new block
        self.on_new_block(&block.header);

        // 2. configure the evm and execute
        let env = self.evm_env_for_block(&block.header, total_difficulty);
        let output = {
            let evm = self.executor.evm_config.evm_with_env(&mut self.state, env);
            // evm.context.external = self.executor.evm_config.default_external_context();
            // //evm.context.external.set_storage(storage);
            // //Prepare storage for wrapper EVM
            // let storage = {
            //     evm.context.external.set_block_hash(block.number, block.mix_hash);
            //     evm.context.external.storage()
            // };
            self.executor.execute_state_transitions(block, evm)
        }?;

        // 3. apply post execution changes
        self.post_execution(block, total_difficulty)?;

        Ok(output)
    }

    /// Apply settings before a new block is executed.
    pub(crate) fn on_new_block(&mut self, header: &Header) {
        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        let state_clear_flag = self.chain_spec().is_spurious_dragon_active_at_block(header.number);
        self.state.set_state_clear_flag(state_clear_flag);
    }

    /// Apply post execution state changes that do not require an [EVM](Evm), such as: block
    /// rewards, withdrawals, and irregular DAO hardfork state change
    pub fn post_execution(
        &mut self,
        block: &BlockWithSenders,
        total_difficulty: U256,
    ) -> Result<(), BlockExecutionError> {
        let mut balance_increments =
            post_block_balance_increments(self.chain_spec(), block, total_difficulty);

        // Irregular state change at Ethereum DAO hardfork
        if self.chain_spec().fork(EthereumHardfork::Dao).transitions_at_block(block.number) {
            // drain balances from hardcoded addresses.
            let drained_balance: u128 = self
                .state
                .drain_balances(DAO_HARDKFORK_ACCOUNTS)
                .map_err(|_| BlockValidationError::IncrementBalanceFailed)?
                .into_iter()
                .sum();

            // return balance to DAO beneficiary.
            *balance_increments.entry(DAO_HARDFORK_BENEFICIARY).or_default() += drained_balance;
        }
        // increment balances
        self.state
            .increment_balances(balance_increments)
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;

        Ok(())
    }
}

impl<EvmConfig, DB> Executor<DB> for ParallelEthBlockExecutor<EvmConfig, DB>
where
    EvmConfig:
        for<'a> ConfigureEvm<Header = Header, DefaultExternalContext<'a> = ParallelEvmContext>,
    DB: Database<Error: Into<ProviderError> + Display>,
{
    type Input<'b> = BlockExecutionInput<'b, BlockWithSenders>;
    type Output = BlockExecutionOutput<Receipt>;
    type Error = BlockExecutionError;

    /// Executes the block and commits the changes to the internal state.
    ///
    /// Returns the receipts of the transactions in the block.
    ///
    /// Returns an error if the block could not be executed or failed verification.
    fn execute(mut self, input: Self::Input<'_>) -> Result<Self::Output, Self::Error> {
        let BlockExecutionInput { block, total_difficulty } = input;
        let ParallelEthExecuteOutput { receipts, requests, gas_used } =
            self.execute_without_verification(block, total_difficulty)?;

        // NOTE: we need to merge keep the reverts for the bundle retention
        self.state.merge_transitions(BundleRetention::Reverts);

        Ok(BlockExecutionOutput { state: self.state.take_bundle(), receipts, requests, gas_used })
    }

    fn execute_with_state_witness<F>(
        mut self,
        input: Self::Input<'_>,
        mut witness: F,
    ) -> Result<Self::Output, Self::Error>
    where
        F: FnMut(&State<DB>),
    {
        let BlockExecutionInput { block, total_difficulty } = input;
        let ParallelEthExecuteOutput { receipts, requests, gas_used } =
            self.execute_without_verification(block, total_difficulty)?;

        // NOTE: we need to merge keep the reverts for the bundle retention
        self.state.merge_transitions(BundleRetention::Reverts);
        witness(&self.state);
        Ok(BlockExecutionOutput { state: self.state.take_bundle(), receipts, requests, gas_used })
    }
}
