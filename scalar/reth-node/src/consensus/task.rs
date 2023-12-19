use futures_util::{future::BoxFuture, stream::Fuse, FutureExt, Stream, StreamExt};
use reth_beacon_consensus::{BeaconEngineMessage, ForkchoiceStatus};
use reth_interfaces::consensus::ForkchoiceState;
use reth_primitives::{
    revm_primitives::HashSet, Block, ChainSpec, IntoRecoveredTransaction, SealedBlockWithSenders,
};
use reth_provider::{CanonChainTracker, CanonStateNotificationSender, Chain, StateProviderFactory};
use reth_stages::PipelineEvent;
use reth_transaction_pool::{TransactionPool, ValidPoolTransaction};
use std::{
    collections::VecDeque,
    f32::consts::E,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, info, warn};

use crate::ExternalTransaction;

use super::{ScalarMiningMode, Storage};

/// A Future that listens for new ready transactions and puts new blocks into storage
pub struct ScalarMiningTask<Client, Pool: TransactionPool> {
    /// The configured chain spec
    chain_spec: Arc<ChainSpec>,
    /// The client used to interact with the state
    client: Client,
    /// The active miner
    miner: ScalarMiningMode,
    /// Single active future that inserts a new block into `storage`
    insert_task: Option<BoxFuture<'static, Option<UnboundedReceiverStream<PipelineEvent>>>>,
    /// Shared storage to insert new blocks
    storage: Storage,
    /// Pool where transactions are stored
    pool: Pool,
    /// backlog of sets of transactions ready to be mined
    queued: VecDeque<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>,
    /// TODO: ideally this would just be a sender of hashes
    to_engine: UnboundedSender<BeaconEngineMessage>,
    /// Used to notify consumers of new blocks
    canon_state_notification: CanonStateNotificationSender,
    /// The pipeline events to listen on
    pipe_line_events: Option<UnboundedReceiverStream<PipelineEvent>>,
    rx_commited_transactions: Fuse<UnboundedReceiverStream<Vec<ExternalTransaction>>>,
    queue_committed_transactions: VecDeque<Vec<ExternalTransaction>>,
    // all transaction nonce in the queue
    queued_txs: HashSet<u64>,
}

// === impl MiningTask ===

impl<Client, Pool: TransactionPool> ScalarMiningTask<Client, Pool> {
    /// Creates a new instance of the task
    pub(crate) fn new(
        chain_spec: Arc<ChainSpec>,
        miner: ScalarMiningMode,
        to_engine: UnboundedSender<BeaconEngineMessage>,
        canon_state_notification: CanonStateNotificationSender,
        storage: Storage,
        client: Client,
        pool: Pool,
        rx_commited_transactions: UnboundedReceiver<Vec<ExternalTransaction>>,
    ) -> Self {
        Self {
            chain_spec,
            client,
            miner,
            insert_task: None,
            storage,
            pool,
            to_engine,
            canon_state_notification,
            queued: Default::default(),
            pipe_line_events: None,
            rx_commited_transactions: UnboundedReceiverStream::new(rx_commited_transactions).fuse(),
            queue_committed_transactions: Default::default(),
            queued_txs: Default::default(),
        }
    }

    /// Sets the pipeline events to listen on.
    pub fn set_pipeline_events(&mut self, events: UnboundedReceiverStream<PipelineEvent>) {
        self.pipe_line_events = Some(events);
    }
}

impl<Client, Pool> Future for ScalarMiningTask<Client, Pool>
where
    Client: StateProviderFactory + CanonChainTracker + Clone + Unpin + 'static,
    Pool: TransactionPool + Unpin + 'static,
    <Pool as TransactionPool>::Transaction: IntoRecoveredTransaction,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // this drives block production and
        loop {
            while let Poll::Ready(Some(commited_transactions)) =
                Pin::new(&mut this.rx_commited_transactions).poll_next(cx)
            {
                this.queue_committed_transactions.push_back(commited_transactions);
            }

            // if let Poll::Ready(transactions) = this.miner.poll(&this.pool, cx) {
            //     // miner returned a set of transaction that we feed to the producer
            //     transactions.into_iter().for_each(|tx| this.queued.push_back(tx));
            // }
            let transactions = this.pool.best_transactions().collect::<Vec<_>>();
            transactions.into_iter().for_each(|tx| {
                if !this.queued_txs.contains(&tx.nonce()) {
                    info!("Push transaction with nonce {:?} into the queue", &tx.nonce());
                    this.queued_txs.insert(tx.nonce());
                    this.queued.push_back(tx);
                }
            });
            if this.insert_task.is_none() {
                if this.queue_committed_transactions.is_empty() {
                    // nothing to insert
                    break;
                }
                let committed_transactions =
                    this.queue_committed_transactions.get(0).expect("Not empty");
                let commited_size = committed_transactions.len();
                let queued_size = this.queued.len();
                info!("Commited size {:?}, queue size {:?}", commited_size, queued_size);
                if queued_size < commited_size {
                    break;
                }
                let mut transactions = vec![];
                info!("Get all transactions from the queue");
                for _ in 0..commited_size {
                    let transaction = this.queued.pop_front().expect("not empty");
                    info!("Transaction nonce {:?}", transaction.nonce());
                    transactions.push(transaction);
                }
                // if queued_size >= commited_size {
                //     // get all transaction from queue
                //     info!("Get all transactions from the queue");
                //     for _ in 0..commited_size {
                //         let transaction = this.queued.pop_front().expect("not empty");
                //         info!("Transaction nonce {:?}", transaction.nonce());
                //         transactions.push(transaction);
                //     }
                // } else {
                //     info!("Get all transactions from the pending pool");
                //     let pooled_transactions = this.pool.best_transactions().collect::<Vec<_>>();
                //     if pooled_transactions.len() > commited_size - queued_size {
                //         for _ in 0..queued_size {
                //             let transaction = this.queued.pop_front().expect("not empty");
                //             info!("Transaction nonce {:?}", transaction.nonce());
                //             transactions.push(transaction);
                //         }
                //         for (index, transaction) in pooled_transactions.into_iter().enumerate() {
                //             if index < commited_size - queued_size {
                //                 info!(
                //                     "Add transaction with nonce {:?} to the new block",
                //                     transaction.nonce()
                //                 );
                //                 transactions.push(transaction);
                //             } else {
                //                 info!(
                //                     "Put the transaction with nonce {:?} to the queue",
                //                     transaction.nonce()
                //                 );
                //                 this.queued.push_back(transaction);
                //             }
                //         }
                //     } else {
                //         for transaction in pooled_transactions.into_iter() {
                //             this.queued.push_back(transaction);
                //         }
                //         warn!("Missing transaction from pending pool. Continue waiting...");
                //         break;
                //     }
                //}
                //If the first transactions in the queue match with committed transactions then process them.
                this.queue_committed_transactions.pop_front();
                // ready to queue in new insert task
                let storage = this.storage.clone();
                let to_engine = this.to_engine.clone();
                let client = this.client.clone();
                let chain_spec = Arc::clone(&this.chain_spec);
                let pool = this.pool.clone();
                let events = this.pipe_line_events.take();
                let canon_state_notification = this.canon_state_notification.clone();

                // Create the mining future that creates a block, notifies the engine that drives
                // the pipeline
                this.insert_task = Some(Box::pin(async move {
                    let mut storage = storage.write().await;

                    let (transactions, senders): (Vec<_>, Vec<_>) = transactions
                        .into_iter()
                        .map(|tx| {
                            let recovered = tx.to_recovered_transaction();
                            let signer = recovered.signer();
                            (recovered.into_signed(), signer)
                        })
                        .unzip();

                    match storage.build_and_execute(transactions.clone(), &client, chain_spec) {
                        Ok((new_header, bundle_state)) => {
                            // clear all transactions from pool
                            // let pool_size_before_execution = pool.best_transactions().count();
                            // let tran_size = transactions.len();
                            let removed_txs = pool.remove_transactions(
                                transactions.iter().map(|tx| tx.hash()).collect(),
                            );
                            // let pool_size = pool.best_transactions().count();
                            // info!(
                            //     "Before size {:?}, txs size {:?} removed txs size {:?} after size {:?}",
                            //     pool_size_before_execution, tran_size, removed_txs.len(), pool_size
                            // );

                            let state = ForkchoiceState {
                                head_block_hash: new_header.hash,
                                finalized_block_hash: new_header.hash,
                                safe_block_hash: new_header.hash,
                            };
                            drop(storage);

                            // TODO: make this a future
                            // await the fcu call rx for SYNCING, then wait for a VALID response
                            loop {
                                // send the new update to the engine, this will trigger the engine
                                // to download and execute the block we just inserted
                                let (tx, rx) = oneshot::channel();
                                let _ = to_engine.send(BeaconEngineMessage::ForkchoiceUpdated {
                                    state,
                                    payload_attrs: None,
                                    tx,
                                });
                                debug!(target: "consensus::auto", ?state, "Sent fork choice update");

                                match rx.await.unwrap() {
                                    Ok(fcu_response) => {
                                        match fcu_response.forkchoice_status() {
                                            ForkchoiceStatus::Valid => break,
                                            ForkchoiceStatus::Invalid => {
                                                error!(target: "consensus::auto", ?fcu_response, "Forkchoice update returned invalid response");
                                                return None;
                                            }
                                            ForkchoiceStatus::Syncing => {
                                                debug!(target: "consensus::auto", ?fcu_response, "Forkchoice update returned SYNCING, waiting for VALID");
                                                // wait for the next fork choice update
                                                continue;
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        error!(target: "consensus::auto", ?err, "Autoseal fork choice update failed");
                                        return None;
                                    }
                                }
                            }

                            // seal the block
                            let block = Block {
                                header: new_header.clone().unseal(),
                                body: transactions,
                                ommers: vec![],
                                withdrawals: None,
                            };
                            let sealed_block = block.seal_slow();

                            let sealed_block_with_senders =
                                SealedBlockWithSenders::new(sealed_block, senders)
                                    .expect("senders are valid");

                            // update canon chain for rpc
                            client.set_canonical_head(new_header.clone());
                            client.set_safe(new_header.clone());
                            client.set_finalized(new_header.clone());

                            debug!(target: "consensus::auto", header=?sealed_block_with_senders.hash(), "sending block notification");

                            let chain =
                                Arc::new(Chain::new(vec![sealed_block_with_senders], bundle_state));

                            // send block notification
                            let _ = canon_state_notification
                                .send(reth_provider::CanonStateNotification::Commit { new: chain });
                        }
                        Err(err) => {
                            warn!(target: "consensus::auto", ?err, "failed to execute block")
                        }
                    }

                    events
                }));
            }

            if let Some(mut fut) = this.insert_task.take() {
                match fut.poll_unpin(cx) {
                    Poll::Ready(events) => {
                        this.pipe_line_events = events;
                    }
                    Poll::Pending => {
                        this.insert_task = Some(fut);
                        break;
                    }
                }
            }
        }

        Poll::Pending
    }
}

impl<Client, Pool: TransactionPool> std::fmt::Debug for ScalarMiningTask<Client, Pool> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarMiningTask").finish_non_exhaustive()
    }
}
