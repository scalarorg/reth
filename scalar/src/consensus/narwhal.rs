// use hyper::body::Bytes;
use jsonrpsee::client_transport::ws::{Url, WsTransportClientBuilder};
use jsonrpsee::core::client::{Client, ClientBuilder, ClientT};
use jsonrpsee::http_client::HttpClientBuilder;
use jsonrpsee::rpc_params;
use jsonrpsee::server::{RpcModule, Server};
use reth_primitives::{IntoRecoveredTransaction, TransactionSigned, TransactionSignedEcRecovered};
use reth_rpc::JwtSecret;
use reth_transaction_pool::{
    NewTransactionEvent, PoolConfig, PoolTransaction, PriceBumpConfig, SubPoolLimit,
    TransactionListenerKind, TransactionPool, ValidPoolTransaction, DEFAULT_PRICE_BUMP,
    REPLACE_BLOB_PRICE_BUMP, TXPOOL_MAX_ACCOUNT_SLOTS_PER_SENDER,
    TXPOOL_SUBPOOL_MAX_SIZE_MB_DEFAULT, TXPOOL_SUBPOOL_MAX_TXS_DEFAULT,
};
use crate::proto::{ConsensusApiClient, ConsensusTransactionIn};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{debug, info};
#[derive(Debug, Clone)]
pub struct ScalarConsensusHandles {}
/// Scalar N&B consensus
///
/// This consensus adapter for listen incommit transaction and send to Consensus Grpc Server.
#[derive(Debug)]
pub struct ScalarConsensus {}
fn create_consensus_transaction<Pool: PoolTransaction + 'static>(
    transaction: Arc<ValidPoolTransaction<Pool>>,
) -> ConsensusTransactionIn {
    let recovered_transaction = transaction.to_recovered_transaction();
    let signed_transaction = recovered_transaction.into_signed();
    let TransactionSigned {
        hash,
        signature,
        transaction,
    } = signed_transaction;
    let tx_bytes = hash.to_vec();
    let sig_bytes = signature.to_bytes(); //[u8;65]
    ConsensusTransactionIn {
        tx_bytes,
        signatures: vec![],
    }
}
impl ScalarConsensus {
    pub async fn new<Pool>(
        socket_addr: SocketAddr,
        pool: &Pool,
        jwt_secret: JwtSecret,
    ) -> eyre::Result<ScalarConsensusHandles>
    where
        Pool: TransactionPool + 'static,
    {
        let handles = ScalarConsensusHandles {};
        let pending_transaction_rx =
            pool.pending_transactions_listener_for(TransactionListenerKind::PropagateOnly);
        let mut transaction_rx =
            pool.new_transactions_listener_for(TransactionListenerKind::PropagateOnly);
        tokio::spawn(async move {
            let url = format!("http://{}", socket_addr);
            let mut client = ConsensusApiClient::connect(url).await.unwrap();
            info!("Connected to the grpc consensus server at {:?}", &socket_addr);
            //let in_stream = tokio_stream::wrappers::ReceiverStream::new(transaction_rx)
            let stream = async_stream::stream! {
                while let Some(NewTransactionEvent { subpool, transaction }) = transaction_rx.recv().await {
                    /*
                     * 231129 TaiVV
                     * Scalar TODO: convert transaction to ConsensusTransactionIn
                     */

                    info!("Receive message from external, send it into narwhal consensus {:?}", &transaction);
                    let consensus_transaction = create_consensus_transaction(transaction);
                    yield consensus_transaction;
                }
            };
            //pin_mut!(stream);
            let stream = Box::pin(stream);
            let response = client.init_transaction(stream).await.unwrap();
            let mut resp_stream = response.into_inner();

            while let Some(received) = resp_stream.next().await {
                let received = received.unwrap();
                info!("\treceived message: `{:?}`", received.payload);
            }
        });
        Ok(handles)
    }

    // Start jsonrpsee client
    async fn start_json_rpc_client() {
        // let middleware = tower::ServiceBuilder::new()
        //     .layer(
        //         TraceLayer::new_for_http()
        //             .on_request(
        //                 |request: &hyper::Request<hyper::Body>, _span: &tracing::Span| tracing::info!(request = ?request, "on_request"),
        //             )
        //             .on_body_chunk(|chunk: &Bytes, latency: Duration, _: &tracing::Span| {
        //                 tracing::info!(size_bytes = chunk.len(), latency = ?latency, "sending body chunk")
        //             })
        //             .make_span_with(DefaultMakeSpan::new().include_headers(true))
        //             .on_response(DefaultOnResponse::new().include_headers(true).latency_unit(LatencyUnit::Micros)),
        //     );

        // let client = HttpClientBuilder::default()
        //     .set_middleware(middleware)
        //     .build(url)?;
    }
}
