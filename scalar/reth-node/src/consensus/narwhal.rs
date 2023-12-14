use crate::proto::{ConsensusApiClient, ExternalTransaction};
use crate::{CommitedTransactions, NAMESPACE};
use reth_primitives::{IntoRecoveredTransaction, TransactionSigned};
use reth_rpc::JwtSecret;
use reth_transaction_pool::{
    NewTransactionEvent, PoolTransaction, TransactionListenerKind, TransactionPool,
    ValidPoolTransaction,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::{debug, error, info};
#[derive(Debug, Clone)]
pub struct ScalarConsensusHandles {}
/// Scalar N&B consensus
///
/// This consensus adapter for listen incommit transaction and send to Consensus Grpc Server.
#[derive(Debug)]
pub struct ScalarConsensus {}
fn create_consensus_transaction<Pool: PoolTransaction + 'static>(
    transaction: Arc<ValidPoolTransaction<Pool>>,
) -> ExternalTransaction {
    let recovered_transaction = transaction.to_recovered_transaction();
    let signed_transaction = recovered_transaction.into_signed();
    let TransactionSigned { hash, signature, transaction } = signed_transaction;
    let tx_bytes = hash.to_vec();
    // let sig_bytes = signature.to_bytes(); //[u8;65]
    ExternalTransaction { namespace: NAMESPACE.to_owned(), tx_bytes }
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
                match received {
                    Ok(CommitedTransactions { transactions }) => {
                        info!("\treceived commited transactions: `{:?}`", &transactions);
                    }
                    Err(err) => {
                        error!("{:?}", err);
                    }
                }
            }
        });
        Ok(handles)
    }
}
