//! Transaction pool arguments

use crate::cli::components::RethNodeComponents;
use crate::cli::ext::RethNodeCommandConfig;
use clap::Args;
use reth_beacon_consensus::BeaconConsensusEngineHandle;
use reth_rpc::JwtSecret;

use crate::consensus::{ScalarConsensus, ScalarConsensusHandles, DEFAULT_NARWHAL_PORT};
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
// impl ConsensusArgs {
//     /// Configures and launches _all_ servers.
//     ///
//     /// Returns the handles for the launched regular RPC server(s) (if any) and the server handle
//     /// for the auth server that handles the `engine_` API that's accessed by the consensus
//     /// layer.
//     #[allow(clippy::too_many_arguments)]
//     pub async fn start_client<Reth, Conf>(
//         &self,
//         components: &Reth, //RethNodeComponentsImpl
//         beacon_consensus_engine_handle: BeaconConsensusEngineHandle,
//         jwt_secret: JwtSecret,
//         _conf: &mut Conf,
//     ) -> eyre::Result<ScalarConsensusHandles>
//     where
//         Reth: RethNodeComponents,
//         Conf: RethNodeCommandConfig,
//     {
//         info!("Start Narwhal&Bullshark consensus client option {:?}", &self.narwhal);
//         if self.narwhal {
//             let socket_address = SocketAddr::new(self.narwhal_addr, self.narwhal_port);
//             let transaction_pool = components.pool();
//             info!("Start Narwhal&Bullshark consensus client at the address {:?}", &socket_address);
//             //ScalarConsensus::new(socket_address, &transaction_pool, beacon_consensus_engine_handle, jwt_secret).await
//         } else {
//             let handles = ScalarConsensusHandles {};
//             Ok(handles)
//         }
//         // let transaction_rx =
//         //     transaction_pool.new_transactions_listener_for(TransactionListenerKind::PropagateOnly);
//         // let pending_transaction_rx = transaction_pool
//         //     .pending_transactions_listener_for(TransactionListenerKind::PropagateOnly);
//     }
// }
