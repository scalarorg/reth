/// Implement STM algorithm for parallel EVM execution.
mod eth_batch_executor;
mod eth_block_executor;
mod eth_evm_executor;
mod provider;
pub use eth_batch_executor::EthBatchExecutor;
pub use eth_block_executor::EthBlockExecutor;
use eth_evm_executor::EthEvmExecutor;
pub use provider::EthExecutorProvider;

#[cfg(test)]
mod test;
