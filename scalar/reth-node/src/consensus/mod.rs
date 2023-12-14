mod args;
mod narwhal;
pub use args::*;
pub use narwhal::*;
/// default grpc consensus component's port
pub const DEFAULT_NARWHAL_PORT: u16 = 8555;
