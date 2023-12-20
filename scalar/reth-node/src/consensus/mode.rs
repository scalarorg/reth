//! The mode the auto seal miner is operating in.

use futures_util::{stream::Fuse, StreamExt};
use reth_primitives::TxHash;
use reth_transaction_pool::{TransactionPool, ValidPoolTransaction};
use std::{
    collections::VecDeque,
    fmt,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::{
        mpsc::{Receiver, UnboundedReceiver},
        RwLock,
    },
    time::Interval,
};
use tokio_stream::{
    wrappers::{ReceiverStream, UnboundedReceiverStream},
    Stream,
};
use tracing::{info, warn};

use crate::ExternalTransaction;

/// Mode of operations for the `Miner`
#[derive(Debug)]
pub enum ScalarMiningMode<Pool: TransactionPool> {
    /// A miner that does nothing
    None,
    /// A miner that listens for new transactions that are ready.
    ///
    /// Either one transaction will be mined per block, or any number of transactions will be
    /// allowed
    Auto(ReadyTransactionMiner),
    /// A miner that listens commited transactions from external Narwhal consensus
    Narwhal(NarwhalTransactionMiner<Pool>),
    /// A miner that constructs a new block every `interval` tick
    FixedBlockTime(FixedBlockTimeMiner),
}

// === impl MiningMode ===

impl<Pool: TransactionPool> ScalarMiningMode<Pool> {
    /// Creates a new instant mining mode that listens for new transactions and tries to build
    /// non-empty blocks as soon as transactions arrive.
    pub fn instant(max_transactions: usize, listener: Receiver<TxHash>) -> Self {
        ScalarMiningMode::Auto(ReadyTransactionMiner {
            max_transactions,
            has_pending_txs: None,
            rx: ReceiverStream::new(listener).fuse(),
        })
    }

    /// Creates a new narwhal mining mode that listens for commited transactions from Narwhal consensus
    /// and tries to build non-empty blocks as soon as transactions arrive.
    pub fn narwhal(
        rx_pending_transaction: Receiver<TxHash>,
        rx_commited_transactions: UnboundedReceiver<Vec<ExternalTransaction>>,
    ) -> Self {
        ScalarMiningMode::Narwhal(NarwhalTransactionMiner {
            max_transactions: usize::MAX,
            // has_pending_txs: None,
            rx_pending_transaction: ReceiverStream::new(rx_pending_transaction).fuse(),
            rx_commited_transactions: UnboundedReceiverStream::new(rx_commited_transactions).fuse(),
            pending_transaction_queue: Default::default(),
            pending_commited_transactions: Default::default(),
        })
    }

    /// Creates a new interval miner that builds a block ever `duration`.
    pub fn interval(duration: Duration) -> Self {
        ScalarMiningMode::FixedBlockTime(FixedBlockTimeMiner::new(duration))
    }

    /// polls the Pool and returns those transactions that should be put in a block, if any.
    pub(crate) fn poll(
        &mut self,
        pool: &Pool,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>>
    where
        Pool: TransactionPool,
    {
        match self {
            ScalarMiningMode::None => Poll::Pending,
            ScalarMiningMode::Auto(miner) => miner.poll(pool, cx),
            ScalarMiningMode::Narwhal(miner) => miner.poll(pool, cx),
            ScalarMiningMode::FixedBlockTime(miner) => miner.poll(pool, cx),
        }
    }
}

/// A miner that's supposed to create a new block every `interval`, mining all transactions that are
/// ready at that time.
///
/// The default blocktime is set to 6 seconds
#[derive(Debug)]
pub struct FixedBlockTimeMiner {
    /// The interval this fixed block time miner operates with
    interval: Interval,
}

// === impl FixedBlockTimeMiner ===

impl FixedBlockTimeMiner {
    /// Creates a new instance with an interval of `duration`
    pub(crate) fn new(duration: Duration) -> Self {
        let start = tokio::time::Instant::now() + duration;
        Self { interval: tokio::time::interval_at(start, duration) }
    }

    fn poll<Pool>(
        &mut self,
        pool: &Pool,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>>
    where
        Pool: TransactionPool,
    {
        if self.interval.poll_tick(cx).is_ready() {
            // drain the pool
            return Poll::Ready(pool.best_transactions().collect());
        }
        Poll::Pending
    }
}

impl Default for FixedBlockTimeMiner {
    fn default() -> Self {
        Self::new(Duration::from_secs(6))
    }
}

/// A miner that Listens for new ready transactions
pub struct ReadyTransactionMiner {
    /// how many transactions to mine per block
    max_transactions: usize,
    /// stores whether there are pending transactions (if known)
    has_pending_txs: Option<bool>,
    /// Receives hashes of transactions that are ready
    rx: Fuse<ReceiverStream<TxHash>>,
}

// === impl ReadyTransactionMiner ===

impl ReadyTransactionMiner {
    fn poll<Pool>(
        &mut self,
        pool: &Pool,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>>
    where
        Pool: TransactionPool,
    {
        // drain the notification stream
        while let Poll::Ready(Some(_hash)) = Pin::new(&mut self.rx).poll_next(cx) {
            self.has_pending_txs = Some(true);
        }

        if self.has_pending_txs == Some(false) {
            return Poll::Pending;
        }

        let transactions = pool.best_transactions().take(self.max_transactions).collect::<Vec<_>>();

        // there are pending transactions if we didn't drain the pool
        self.has_pending_txs = Some(transactions.len() >= self.max_transactions);

        if transactions.is_empty() {
            return Poll::Pending;
        }

        Poll::Ready(transactions)
    }
}

impl fmt::Debug for ReadyTransactionMiner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadyTransactionMiner")
            .field("max_transactions", &self.max_transactions)
            .finish_non_exhaustive()
    }
}

/// A miner that Listens for new ready transactions
pub struct NarwhalTransactionMiner<Pool: TransactionPool> {
    /// how many transactions to mine per block
    max_transactions: usize,
    // /// stores whether there are pending transactions (if known)
    // has_pending_txs: Option<bool>,
    /// Receives hashes of transactions that are ready
    rx_pending_transaction: Fuse<ReceiverStream<TxHash>>,
    rx_commited_transactions: Fuse<UnboundedReceiverStream<Vec<ExternalTransaction>>>,
    pending_transaction_queue:
        VecDeque<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>,
    pending_commited_transactions: VecDeque<Vec<ExternalTransaction>>,
}

// === impl NarwhalTransactionMiner ===

impl<Pool: TransactionPool> NarwhalTransactionMiner<Pool> {
    fn poll(
        &mut self,
        pool: &Pool,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>>
    where
        Pool: TransactionPool,
    {
        // drain the notification stream
        while let Poll::Ready(Some(tx_hash)) =
            Pin::new(&mut self.rx_pending_transaction).poll_next(cx)
        {
            //let guard = self.pending_transaction_queue.write().await;
            if let Some(transaction) = pool.get(&tx_hash) {
                info!("Pending transaction {:?}", transaction.nonce());
                self.pending_transaction_queue.push_back(transaction);
            }
        }

        while let Poll::Ready(Some(comitted_transactions)) =
            Pin::new(&mut self.rx_commited_transactions).poll_next(cx)
        {
            self.pending_commited_transactions.push_back(comitted_transactions);
        }
        let mut transactions = vec![];
        // let transactions = pool.best_transactions().collect::<Vec<_>>();

        // there are pending transactions if we didn't drain the pool
        // self.has_pending_txs = Some(transactions.len() >= self.max_transactions);
        if let Some(commited_transactions) = self.pending_commited_transactions.get(0) {
            for transaction in commited_transactions.iter() {
                //Find transaction in the pending queue with current hash
                let mut index = 0;
                let mut found = false;
                loop {
                    if let Some(tran) = self.pending_transaction_queue.get(index) {
                        if tran.hash().as_slice() == transaction.tx_bytes.as_slice() {
                            transactions.push(Arc::clone(tran));
                            info!(
                                "Found transaction with nonce {:?} and hash {:?}",
                                tran.nonce(),
                                &transaction.tx_bytes
                            );
                            found = true;
                            break;
                        } else {
                            index += 1;
                        }
                    } else {
                        break;
                    }
                }
                if found {
                    self.pending_transaction_queue.remove(index);
                } else {
                    warn!("Missing transaction with hash {:?}", &transaction.tx_bytes);
                    return Poll::Pending;
                }
            }

            // for tran in commited_transactions.iter() {
            //     let queue_tran = self.pending_transaction_queue.pop_front().expect("non empty");
            //     tracing::info!("Next queued transaction nonce {:?}", queue_tran.nonce());
            //     if tran.tx_bytes.as_slice() != queue_tran.hash().as_slice() {
            //         tracing::warn!(
            //             "Transaction {} with commited hash are not match",
            //             queue_tran.nonce()
            //         );
            //     }
            //     transactions.push(queue_tran);
            // }
            self.pending_commited_transactions.pop_front();
            if transactions.is_empty() {
                return Poll::Pending;
            }
            return Poll::Ready(transactions);
        } else {
            return Poll::Pending;
        }
    }
}

impl<Pool: TransactionPool> fmt::Debug for NarwhalTransactionMiner<Pool> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NarwhalTransactionMiner")
            .field("max_transactions", &self.max_transactions)
            .finish_non_exhaustive()
    }
}
