use crate::proto::{ConsensusApiClient, ExternalTransaction};
use crate::{CommitedTransactions, NAMESPACE};
use backon::{ExponentialBuilder, Retryable};
use reth_beacon_consensus::{BeaconConsensusEngineHandle, BeaconEngineMessage};
use reth_interfaces::consensus::{Consensus, ConsensusError};
use reth_primitives::{
    ChainSpec, Header, IntoRecoveredTransaction, SealedBlock, SealedHeader, TransactionSigned,
    TxHash, U256,
};
use reth_provider::{BlockReaderIdExt, CanonStateNotificationSender};
use reth_rpc_types::{ExecutionPayload, ExecutionPayloadV1, ExecutionPayloadV2, Withdrawal};
use reth_transaction_pool::{
    NewTransactionEvent, PoolTransaction, TransactionListenerKind, TransactionPool,
    ValidPoolTransaction,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::{self, Receiver, UnboundedReceiver, UnboundedSender};
use tokio_stream::StreamExt;
use tracing::{debug, error, info};

use super::{ConsensusArgs, ScalarClient, ScalarMiningMode, ScalarMiningTask, Storage};
#[derive(Debug, Clone)]
pub struct ScalarConsensusHandles {}
/// Scalar N&B consensus
///
/// This consensus adapter for listen incommit transaction and send to Consensus Grpc Server.
#[derive(Debug, Clone)]
pub struct ScalarConsensus {
    /// Configuration
    chain_spec: Arc<ChainSpec>,
    //beacon_consensus_engine_handle: BeaconConsensusEngineHandle,
}

impl ScalarConsensus {
    /// Create a new instance of [AutoSealConsensus]
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { chain_spec }
    }
}

impl Consensus for ScalarConsensus {
    fn validate_header(&self, _header: &SealedHeader) -> Result<(), ConsensusError> {
        Ok(())
    }

    fn validate_header_against_parent(
        &self,
        _header: &SealedHeader,
        _parent: &SealedHeader,
    ) -> Result<(), ConsensusError> {
        Ok(())
    }

    fn validate_header_with_total_difficulty(
        &self,
        _header: &Header,
        _total_difficulty: U256,
    ) -> Result<(), ConsensusError> {
        Ok(())
    }

    fn validate_block(&self, _block: &SealedBlock) -> Result<(), ConsensusError> {
        Ok(())
    }
}

/// Builder type for configuring the setup
#[derive(Debug)]
pub struct ScalarBuilder<Client, Pool: TransactionPool> {
    client: Client,
    consensus: ScalarConsensus,
    pool: Pool,
    mode: ScalarMiningMode<Pool>,
    storage: Storage,
    to_engine: UnboundedSender<BeaconEngineMessage>,
    canon_state_notification: CanonStateNotificationSender,
    tx_commited_transactions: UnboundedSender<Vec<ExternalTransaction>>,
    consensus_args: ConsensusArgs,
}

// === impl AutoSealBuilder ===

impl<Client, Pool: TransactionPool + 'static> ScalarBuilder<Client, Pool>
where
    Client: BlockReaderIdExt,
{
    /// Creates a new builder instance to configure all parts.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        client: Client,
        pool: Pool,
        mode: ScalarMiningMode<Pool>,
        to_engine: UnboundedSender<BeaconEngineMessage>,
        canon_state_notification: CanonStateNotificationSender,
        tx_commited_transactions: UnboundedSender<Vec<ExternalTransaction>>,
        consensus_args: ConsensusArgs,
    ) -> Self {
        let latest_header = client
            .latest_header()
            .ok()
            .flatten()
            .unwrap_or_else(|| chain_spec.sealed_genesis_header());
        Self {
            storage: Storage::new(latest_header),
            client,
            consensus: ScalarConsensus::new(chain_spec),
            pool,
            mode,
            to_engine,
            canon_state_notification,
            tx_commited_transactions,
            consensus_args,
        }
    }

    /// Sets the [MiningMode] it operates in, default is [MiningMode::Auto]
    pub fn mode(mut self, mode: ScalarMiningMode<Pool>) -> Self {
        self.mode = mode;
        self
    }

    /// Consumes the type and returns all components
    #[track_caller]
    pub fn build(self) -> (ScalarConsensus, ScalarClient, ScalarMiningTask<Client, Pool>) {
        let Self {
            client,
            consensus,
            pool,
            mode,
            storage,
            to_engine,
            canon_state_notification,
            tx_commited_transactions,
            consensus_args,
        } = self;
        let auto_client = ScalarClient::new(storage.clone());

        let tx_pool = pool.clone();

        let task = ScalarMiningTask::new(
            Arc::clone(&consensus.chain_spec),
            mode,
            to_engine,
            canon_state_notification,
            storage,
            client,
            pool,
        );
        /*
         * 2023-12-15 Taivv
         * Start consensus client to the  Narwhal layer
         */
        let ConsensusArgs { narwhal_addr, narwhal_port, .. } = consensus_args;
        let socket_address = SocketAddr::new(narwhal_addr, narwhal_port);

        tokio::spawn(async move {
            (move || {
                start_consensus_client(
                    socket_address,
                    tx_pool.clone(),
                    tx_commited_transactions.clone(),
                )
            })
            .retry(&ExponentialBuilder::default().with_max_times(100_000))
            .notify(|err, _| error!("Scalar consensus client error: {}. Retrying...", err))
            .await
        });
        (consensus, auto_client, task)
    }
}

async fn start_consensus_client<Pool>(
    socket_addr: SocketAddr,
    tx_pool: Pool,
    tx_commited_transactions: UnboundedSender<Vec<ExternalTransaction>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    Pool: TransactionPool + 'static,
{
    let url = format!("http://{}", socket_addr);
    let mut client = ConsensusApiClient::connect(url).await?;
    //let handler = ScalarConsensus { beacon_consensus_engine_handle };
    info!("Connected to the grpc consensus server at {:?}", &socket_addr);
    let mut rx_pending_transaction = tx_pool.pending_transactions_listener();
    //let in_stream = tokio_stream::wrappers::ReceiverStream::new(transaction_rx)
    let stream = async_stream::stream! {
        while let Some(tx_hash) = rx_pending_transaction.recv().await {
            /*
             * 231129 TaiVV
             * Scalar TODO: convert transaction to ConsensusTransactionIn
             */
            let tx_bytes = tx_hash.as_slice().to_vec();
            info!("Receive a pending transaction hash, send it into narwhal consensus {:?}", &tx_bytes);
            let consensus_transaction = ExternalTransaction { namespace: NAMESPACE.to_owned(), tx_bytes };
            yield consensus_transaction;
        }
    };
    //pin_mut!(stream);
    let stream = Box::pin(stream);
    let response = client.init_transaction(stream).await?;
    let mut resp_stream = response.into_inner();

    while let Some(received) = resp_stream.next().await {
        match received {
            Ok(CommitedTransactions { transactions }) => {
                info!("Received {:?} commited transactions.", transactions.len());
                if let Err(err) = tx_commited_transactions.send(transactions) {
                    error!("{:?}", err);
                }
                //let _ = handler.handle_commited_transactions(transactions).await;
            }
            Err(err) => {
                return Err(Box::new(err));
            }
        }
    }

    Ok(())
}

// fn create_consensus_transaction<Pool: PoolTransaction + 'static>(
//     transaction: Arc<ValidPoolTransaction<Pool>>,
// ) -> ExternalTransaction {
//     let recovered_transaction = transaction.to_recovered_transaction();
//     let signed_transaction = recovered_transaction.into_signed();
//     let TransactionSigned { hash, signature, transaction } = signed_transaction;
//     let tx_bytes = hash.to_vec();
//     // let sig_bytes = signature.to_bytes(); //[u8;65]
//     ExternalTransaction { namespace: NAMESPACE.to_owned(), tx_bytes }
// }
