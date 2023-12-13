//! Transaction pool arguments

use crate::cli::components::RethNodeComponents;
use crate::cli::ext::RethNodeCommandConfig;
use clap::Args;
use reth_rpc::JwtSecret;
use reth_transaction_pool::{
    PoolConfig, PriceBumpConfig, SubPoolLimit, TransactionListenerKind, TransactionPool,
    DEFAULT_PRICE_BUMP, REPLACE_BLOB_PRICE_BUMP, TXPOOL_MAX_ACCOUNT_SLOTS_PER_SENDER,
    TXPOOL_SUBPOOL_MAX_SIZE_MB_DEFAULT, TXPOOL_SUBPOOL_MAX_TXS_DEFAULT,
};

use crate::consensus::{ScalarConsensusHandles, ScalarConsensus, DEFAULT_NARWHAL_PORT};
use std::net::SocketAddr;
use std::net::{IpAddr, Ipv4Addr};
use tracing::info;

/// Parameters for debugging purposes
#[derive(Debug, Clone, Args)]
pub struct ConsensusArgs {
    /// Enable the Narwhal client
    #[arg(long, default_value_if("dev", "true", "true"))]
    pub narwhal: bool,

    /// Http server address to listen on
    #[arg(long = "narwhal.addr", default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    pub narwhal_addr: IpAddr,

    /// Http server port to listen on
    #[arg(long = "narwhal.port", default_value_t = DEFAULT_NARWHAL_PORT)]
    pub narwhal_port: u16,
}
impl ConsensusArgs {
    /// Configures and launches _all_ servers.
    ///
    /// Returns the handles for the launched regular RPC server(s) (if any) and the server handle
    /// for the auth server that handles the `engine_` API that's accessed by the consensus
    /// layer.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_client<Reth, Conf>(
        &self,
        components: &Reth, //RethNodeComponentsImpl
        jwt_secret: JwtSecret,
        _conf: &mut Conf,
    ) -> eyre::Result<ScalarConsensusHandles>
    where
        Reth: RethNodeComponents,
        Conf: RethNodeCommandConfig,
    {
        info!(
            "Start Narwhal&Bullshark consensus client option {:?}",
            &self.narwhal
        );
        if self.narwhal {
            let socket_address = SocketAddr::new(self.narwhal_addr, self.narwhal_port);
            let transaction_pool = components.pool();
            info!(
                "Start Narwhal&Bullshark consensus client at the address {:?}",
                &socket_address
            );
            ScalarConsensus::new(
                socket_address,
                &transaction_pool,
                jwt_secret,
            )
            .await
        } else {
            let handles = ScalarConsensusHandles {};
            Ok(handles)
        }
        // let transaction_rx =
        //     transaction_pool.new_transactions_listener_for(TransactionListenerKind::PropagateOnly);
        // let pending_transaction_rx = transaction_pool
        //     .pending_transactions_listener_for(TransactionListenerKind::PropagateOnly);
    }
}
// impl ConsensusArgs {
//     /// Returns transaction pool configuration.
//     pub fn consensus_config(&self) -> ConsensusConfig {
//         ConsensusConfig {
//             pending_limit: SubPoolLimit {
//                 max_txs: self.pending_max_count,
//                 max_size: self.pending_max_size * 1024 * 1024,
//             },
//             basefee_limit: SubPoolLimit {
//                 max_txs: self.basefee_max_count,
//                 max_size: self.basefee_max_size * 1024 * 1024,
//             },
//             queued_limit: SubPoolLimit {
//                 max_txs: self.queued_max_count,
//                 max_size: self.queued_max_size * 1024 * 1024,
//             },
//             max_account_slots: self.max_account_slots,
//             price_bumps: PriceBumpConfig {
//                 default_price_bump: self.price_bump,
//                 replace_blob_tx_price_bump: self.blob_transaction_price_bump,
//             },
//         }
//     }
// }
