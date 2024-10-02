//! Ethereum evm executor.

use alloc::{boxed::Box, vec, vec::Vec};
use core::fmt::Display;
use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_evm::{
    execute::{BlockExecutionError, BlockValidationError, ProviderError},
    system_calls::{
        apply_beacon_root_contract_call, apply_blockhashes_contract_call,
        apply_consolidation_requests_contract_call, apply_withdrawal_requests_contract_call,
    },
    ConfigureEvm,
};
use reth_primitives::{BlockWithSenders, Header, Receipt, Request};
use reth_revm::{db::State, Evm};
use revm_primitives::{
    db::{Database, DatabaseCommit},
    ResultAndState,
};
use std::sync::{Arc, Mutex, OnceLock};

use crate::index_mutex;

use super::{
    chain::PevmChain,
    evm::{EvmWrapper, ExecutionError, PevmTxExecutionResult, VmExecutionError, VmExecutionResult},
    storage::Storage,
    types::{Task, TxVersion},
    Scheduler,
};

#[derive(Debug)]
pub(super) enum AbortReason {
    FallbackToSequential,
    ExecutionError(ExecutionError),
}

/// Helper type for the output of executing a block.
#[derive(Debug, Clone)]
pub(super) struct ParallelEthExecuteOutput {
    pub(super) receipts: Vec<Receipt>,
    pub(super) requests: Vec<Request>,
    pub(super) gas_used: u64,
}

/// Helper container type for EVM with chain spec.
#[derive(Default, Debug)]
pub(super) struct ParallelEthEvmExecutor<EvmConfig> {
    /// The chainspec
    pub(super) chain_spec: Arc<ChainSpec>,
    /// How to create an EVM.
    pub(super) evm_config: EvmConfig,
    pub(super) abort_reason: OnceLock<AbortReason>,
    pub(super) execution_results: Vec<Mutex<Option<PevmTxExecutionResult>>>,
}

impl<EvmConfig> ParallelEthEvmExecutor<EvmConfig>
where
    EvmConfig: ConfigureEvm<Header = Header>,
{
    /// Executes the transactions in the block and returns the receipts of the transactions in the
    /// block, the total gas used and the list of EIP-7685 [requests](Request).
    ///
    /// This applies the pre-execution and post-execution changes that require an [EVM](Evm), and
    /// executes the transactions.
    ///
    /// # Note
    ///
    /// It does __not__ apply post-execution changes that do not require an [EVM](Evm), for that see
    /// [`EthBlockExecutor::post_execution`].
    pub(super) fn execute_state_transitions<Ext, DB>(
        &self,
        block: &BlockWithSenders,
        mut evm: Evm<'_, Ext, &mut State<DB>>,
    ) -> Result<ParallelEthExecuteOutput, BlockExecutionError>
    where
        DB: Database,
        DB::Error: Into<ProviderError> + Display,
    {
        // apply pre execution changes
        apply_beacon_root_contract_call(
            &self.evm_config,
            &self.chain_spec,
            block.timestamp,
            block.number,
            block.parent_beacon_block_root,
            &mut evm,
        )?;
        apply_blockhashes_contract_call(
            &self.evm_config,
            &self.chain_spec,
            block.timestamp,
            block.number,
            block.parent_hash,
            &mut evm,
        )?;

        // The merge from September 15, 2022
        self.evm_config.fill_block_env(evm.block_mut(), &block.header, true);
        // Create transaction environments
        let tx_envs = block
            .transactions_with_sender()
            .map(|(address, transaction)| {
                self.evm_config.fill_tx_env(evm.tx_mut(), transaction, *address);
                evm.tx().clone()
            })
            .collect::<Vec<_>>();
        // Use tokio runtime to execute transactions in parallel
        let mut cumulative_gas_used = 0;
        let mut receipts = Vec::with_capacity(block.body.transactions.len());
        for (sender, transaction) in block.transactions_with_sender() {
            // The sum of the transaction’s gas limit, Tg, and the gas utilized in this block prior,
            // must be no greater than the block’s gasLimit.
            let block_available_gas = (block.header.gas_limit - cumulative_gas_used) as u64;
            if transaction.gas_limit() > block_available_gas {
                return Err(BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas {
                    transaction_gas_limit: transaction.gas_limit(),
                    block_available_gas,
                }
                .into());
            }

            self.evm_config.fill_tx_env(evm.tx_mut(), transaction, *sender);

            // Execute transaction.
            let ResultAndState { result, state } = evm.transact().map_err(move |err| {
                let new_err = err.map_db_err(|e| e.into());
                // Ensure hash is calculated for error log, if not already done
                BlockValidationError::EVM {
                    hash: transaction.recalculate_hash(),
                    error: Box::new(new_err),
                }
            })?;
            evm.db_mut().commit(state);

            // append gas used
            cumulative_gas_used += result.gas_used() as u128;

            // Push transaction changeset and calculate header bloom filter for receipt.
            receipts.push(
                #[allow(clippy::needless_update)] // side-effect of optimism fields
                Receipt {
                    tx_type: transaction.tx_type(),
                    // Success flag was added in `EIP-658: Embedding transaction status code in
                    // receipts`.
                    success: result.is_success(),
                    cumulative_gas_used: cumulative_gas_used as u64,
                    // convert to reth log
                    logs: result.into_logs(),
                    ..Default::default()
                },
            );
        }

        let requests = if self.chain_spec.is_prague_active_at_timestamp(block.timestamp) {
            // Collect all EIP-6110 deposits
            let deposit_requests =
                crate::eip6110::parse_deposits_from_receipts(&self.chain_spec, &receipts)?;

            // Collect all EIP-7685 requests
            let withdrawal_requests =
                apply_withdrawal_requests_contract_call(&self.evm_config, &mut evm)?;

            // Collect all EIP-7251 requests
            let consolidation_requests =
                apply_consolidation_requests_contract_call(&self.evm_config, &mut evm)?;

            [deposit_requests, withdrawal_requests, consolidation_requests].concat()
        } else {
            vec![]
        };

        Ok(ParallelEthExecuteOutput { receipts, requests, gas_used: cumulative_gas_used as u64 })
    }
    fn try_execute<'a, S: Storage, C: PevmChain>(
        &self,
        vm: &EvmWrapper<'a, S, C>,
        scheduler: &Scheduler,
        tx_version: TxVersion,
    ) -> Option<Task> {
        loop {
            return match vm.execute(&tx_version) {
                Err(VmExecutionError::Retry) => {
                    if self.abort_reason.get().is_none() {
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::FallbackToSequential) => {
                    scheduler.abort();
                    self.abort_reason.get_or_init(|| AbortReason::FallbackToSequential);
                    None
                }
                Err(VmExecutionError::Blocking(blocking_tx_idx)) => {
                    if !scheduler.add_dependency(tx_version.tx_idx, blocking_tx_idx)
                        && self.abort_reason.get().is_none()
                    {
                        // Retry the execution immediately if the blocking transaction was
                        // re-executed by the time we can add it as a dependency.
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::ExecutionError(err)) => {
                    scheduler.abort();
                    self.abort_reason.get_or_init(|| AbortReason::ExecutionError(err));
                    None
                }
                Ok(VmExecutionResult { execution_result, flags }) => {
                    *index_mutex!(self.execution_results, tx_version.tx_idx) =
                        Some(execution_result);
                    scheduler.finish_execution(tx_version, flags)
                }
            };
        }
    }
}
