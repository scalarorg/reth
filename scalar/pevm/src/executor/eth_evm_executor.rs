//! Ethereum evm executor.

use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
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

/// Helper type for the output of executing a block.
#[derive(Debug, Clone)]
pub(super) struct EthExecuteOutput {
    pub(super) receipts: Vec<Receipt>,
    pub(super) requests: Vec<Request>,
    pub(super) gas_used: u64,
}

/// Helper container type for EVM with chain spec.
#[derive(Debug, Clone)]
pub(super) struct EthEvmExecutor<EvmConfig> {
    /// The chainspec
    pub(super) chain_spec: Arc<ChainSpec>,
    /// How to create an EVM.
    pub(super) evm_config: EvmConfig,
}

impl<EvmConfig> EthEvmExecutor<EvmConfig>
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
    ) -> Result<EthExecuteOutput, BlockExecutionError>
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

        // execute transactions
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

        Ok(EthExecuteOutput { receipts, requests, gas_used: cumulative_gas_used as u64 })
    }
}