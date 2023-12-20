mod args;
mod client;
mod mode;
mod scalar;
mod storage;
mod task;
pub use args::*;
pub use client::*;
pub use mode::*;
pub use scalar::*;
pub use storage::*;
pub use task::*;
/// default grpc consensus component's port
pub const DEFAULT_NARWHAL_PORT: u16 = 8555;
/// default block time
pub const DEFAULT_BLOCK_TIME: u32 = 12000;
/// Maximum number of transactions in each block
pub const DEFAULT_MAX_BLOCK_TRANSACTIONS: u32 = 2000;
