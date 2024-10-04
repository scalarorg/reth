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
use reth_execution_errors::InternalBlockExecutionError;
use reth_primitives::{BlockWithSenders, Header, Receipt, Request};
use reth_revm::{db::State, Evm};
use revm_primitives::{
    db::{Database, DatabaseCommit},
    ResultAndState, TxEnv,
};
use std::{
    num::NonZeroUsize,
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
};
use tokio::{
    io,
    runtime::{Builder, Runtime},
    task::JoinHandle,
};

use crate::index_mutex;

use super::{
    chain::PevmChain,
    context::ParallelEvmContextTrait,
    evm::{EvmWrapper, ExecutionError, PevmTxExecutionResult, VmExecutionError, VmExecutionResult},
    memory::MvMemory,
    scheduler,
    storage::{BlockHashes, InMemoryStorage, Storage},
    types::{BuildIdentityHasher, Task, TxVersion},
    ParallelEvmContext, Scheduler,
};
/// Errors when executing a block with pevm.
#[derive(Debug, Clone, PartialEq)]
pub enum PevmError<C: PevmChain> {
    /// Cannot derive the chain spec from the block header.
    BlockSpecError(C::BlockSpecError),
    /// Transactions lack information for execution.
    MissingTransactionData,
    /// Invalid input transaction.
    InvalidTransaction(C::TransactionParsingError),
    /// Storage error.
    // TODO: More concrete types than just an arbitrary string.
    StorageError(String),
    /// EVM execution error.
    // TODO: More concrete types than just an arbitrary string.
    ExecutionError(String),
    /// Impractical errors that should be unreachable.
    /// The library has bugs if this is yielded.
    UnreachableError,
}
/// Execution result of a block
pub type PevmResult<C> = Result<Vec<PevmTxExecutionResult>, PevmError<C>>;

#[derive(Debug)]
pub(super) enum AbortReason {
    FallbackToSequential,
    ExecutionError(ExecutionError),
}

// TODO: Better implementation
#[derive(Debug)]
struct AsyncDropper<T> {
    sender: mpsc::Sender<T>,
    _handle: thread::JoinHandle<()>,
}

impl<T: Send + 'static> Default for AsyncDropper<T> {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self { sender, _handle: std::thread::spawn(move || receiver.into_iter().for_each(drop)) }
    }
}

impl<T> AsyncDropper<T> {
    fn drop(&self, t: T) {
        // TODO: Better error handling
        self.sender.send(t).unwrap();
    }
}

/// Helper type for the output of executing a block.
#[derive(Debug, Clone)]
pub(super) struct ParallelEthExecuteOutput {
    pub(super) receipts: Vec<Receipt>,
    pub(super) requests: Vec<Request>,
    pub(super) gas_used: u64,
}

/// Helper container type for EVM with chain spec.
#[derive(Debug)]
pub(super) struct ParallelEthEvmExecutor<EvmConfig> {
    /// The chainspec
    pub(super) chain_spec: Arc<ChainSpec>,
    /// How to create an EVM.
    pub(super) evm_config: EvmConfig,
    pub(super) hasher: ahash::RandomState,
    pub(super) execution_results: Vec<Mutex<Option<PevmTxExecutionResult>>>,
    pub(super) abort_reason: OnceLock<AbortReason>,
    pub(super) dropper: AsyncDropper<(MvMemory, Scheduler, Vec<TxEnv>)>,
}
impl<EvmConfig> ParallelEthEvmExecutor<EvmConfig> {
    pub fn new(chain_spec: Arc<ChainSpec>, evm_config: EvmConfig) -> Self {
        Self {
            chain_spec,
            evm_config,
            abort_reason: OnceLock::new(),
            execution_results: Vec::new(),
            hasher: ahash::RandomState::new(),
            dropper: AsyncDropper::default(),
        }
    }
}
impl<EvmConfig> ParallelEthEvmExecutor<EvmConfig>
where
    EvmConfig: ConfigureEvm<Header = Header>,
{
    #[inline]
    fn get_concurrency_level() -> usize {
        // This max should be tuned to the running machine,
        // ideally also per block depending on the number of
        // transactions, gas usage, etc. ARM machines seem to
        // go higher thanks to their low thread overheads.
        let concurent_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN).min(
            NonZeroUsize::new(
                #[cfg(target_arch = "aarch64")]
                12,
                #[cfg(not(target_arch = "aarch64"))]
                8,
            )
            .unwrap(),
        );
        concurent_level.get()
    }
    /// Creates tokio runtime based on the available parallelism.
    fn prepare_runtime(&self, workers: usize) -> io::Result<Runtime> {
        Builder::new_multi_thread()
            .worker_threads(workers)
            .thread_name("parallel_evm")
            .thread_stack_size(3 * 1024 * 1024)
            .build()
    }
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
        Ext: ParallelEvmContextTrait,
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
        let block_env = evm.block_mut();
        self.evm_config.fill_block_env(block_env, &block.header, true);
        // Create transaction environments
        let tx_envs = block
            .transactions_with_sender()
            .map(|(address, transaction)| {
                self.evm_config.fill_tx_env(evm.tx_mut(), transaction, *address);
                evm.tx().clone()
            })
            .collect::<Vec<_>>();
        // Use tokio runtime to execute transactions in parallel
        let concurrency_level = Self::get_concurrency_level();
        let runtime: Runtime = self.prepare_runtime(concurrency_level).map_err(|err| {
            BlockExecutionError::Internal(InternalBlockExecutionError::Other(Box::new(err)))
        })?;
        //Initialize empty memory for WrapperEvm
        let mv_memory = MvMemory::new(tx_envs.len(), [], []);
        let block_size = tx_envs.len();
        let scheduler = Scheduler::new(block_size);
        //Prepare storage for wrapper EVM
        let storage = preapre_internal_storage(&mut evm, block);
        let evm = EvmWrapper::new(
            &self.hasher,
            storage,
            &mv_memory,
            chain,
            evm.block(),
            &tx_envs,
            evm.spec_id(),
        );

        for _ in 0..concurrency_level {
            runtime.spawn(async {
                let mut task = scheduler.next_task();
                while task.is_some() {
                    task = match task.unwrap() {
                        Task::Execution(tx_version) => {
                            self.try_execute(&evm, &scheduler, tx_version)
                        }
                        Task::Validation(tx_version) => {
                            try_validate(&mv_memory, &scheduler, &tx_version)
                        }
                    };

                    // TODO: Have different functions or an enum for the caller to choose
                    // the handling behaviour when a transaction's EVM execution fails.
                    // Parallel block builders would like to exclude such transaction,
                    // verifiers may want to exit early to save CPU cycles, while testers
                    // may want to collect all execution results. We are exiting early as
                    // the default behaviour for now.
                    if self.abort_reason.get().is_some() {
                        break;
                    }

                    if task.is_none() {
                        task = scheduler.next_task();
                    }
                }
            });
        }

        // let mut cumulative_gas_used = 0;
        // let mut receipts = Vec::with_capacity(block.body.transactions.len());
        // for (sender, transaction) in block.transactions_with_sender() {
        //     // The sum of the transaction’s gas limit, Tg, and the gas utilized in this block prior,
        //     // must be no greater than the block’s gasLimit.
        //     let block_available_gas = (block.header.gas_limit - cumulative_gas_used) as u64;
        //     if transaction.gas_limit() > block_available_gas {
        //         return Err(BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas {
        //             transaction_gas_limit: transaction.gas_limit(),
        //             block_available_gas,
        //         }
        //         .into());
        //     }

        //     self.evm_config.fill_tx_env(evm.tx_mut(), transaction, *sender);

        //     // Execute transaction.
        //     let ResultAndState { result, state } = evm.transact().map_err(move |err| {
        //         let new_err = err.map_db_err(|e| e.into());
        //         // Ensure hash is calculated for error log, if not already done
        //         BlockValidationError::EVM {
        //             hash: transaction.recalculate_hash(),
        //             error: Box::new(new_err),
        //         }
        //     })?;
        //     evm.db_mut().commit(state);

        //     // append gas used
        //     cumulative_gas_used += result.gas_used() as u128;

        //     // Push transaction changeset and calculate header bloom filter for receipt.
        //     receipts.push(
        //         #[allow(clippy::needless_update)] // side-effect of optimism fields
        //         Receipt {
        //             tx_type: transaction.tx_type(),
        //             // Success flag was added in `EIP-658: Embedding transaction status code in
        //             // receipts`.
        //             success: result.is_success(),
        //             cumulative_gas_used: cumulative_gas_used as u64,
        //             // convert to reth log
        //             logs: result.into_logs(),
        //             ..Default::default()
        //         },
        //     );
        // }

        // let requests = if self.chain_spec.is_prague_active_at_timestamp(block.timestamp) {
        //     // Collect all EIP-6110 deposits
        //     let deposit_requests =
        //         crate::eip6110::parse_deposits_from_receipts(&self.chain_spec, &receipts)?;

        //     // Collect all EIP-7685 requests
        //     let withdrawal_requests =
        //         apply_withdrawal_requests_contract_call(&self.evm_config, &mut evm)?;

        //     // Collect all EIP-7251 requests
        //     let consolidation_requests =
        //         apply_consolidation_requests_contract_call(&self.evm_config, &mut evm)?;

        //     [deposit_requests, withdrawal_requests, consolidation_requests].concat()
        // } else {
        //     vec![]
        // };

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
/// A common inmemory storage is stored in the EVM external context
/// Inject current block hash into the storage
/// Then return reference to the storage
fn preapre_internal_storage<'a, Ext, DB>(
    evm: &'a mut Evm<'_, Ext, &mut State<DB>>,
    block: &BlockWithSenders,
) -> &'a InMemoryStorage<'a>
where
    Ext: ParallelEvmContextTrait,
    DB: Database,
    DB::Error: Into<ProviderError> + Display,
{
    evm.context.external.set_block_hash(block.number, block.parent_hash);
    evm.context.external.storage()
}
fn try_validate(
    mv_memory: &MvMemory,
    scheduler: &Scheduler,
    tx_version: &TxVersion,
) -> Option<Task> {
    let read_set_valid = mv_memory.validate_read_locations(tx_version.tx_idx);
    let aborted = !read_set_valid && scheduler.try_validation_abort(tx_version);
    if aborted {
        mv_memory.convert_writes_to_estimates(tx_version.tx_idx);
    }
    scheduler.finish_validation(tx_version, aborted)
}
