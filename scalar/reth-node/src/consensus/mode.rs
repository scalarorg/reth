//! The mode the auto seal miner is operating in.

use crate::ExternalTransaction;
use futures_util::{stream::Fuse, StreamExt};
use reth_db::mdbx::tx;
use reth_primitives::{revm_primitives::HashSet, TxHash};
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
    sync::mpsc::{Receiver, UnboundedReceiver},
    time::Interval,
};
use tokio_stream::{
    wrappers::{ReceiverStream, UnboundedReceiverStream},
    Stream,
};
use tower_http::follow_redirect::policy::PolicyExt;
use tracing::info;

/// Mode of operations for the `Miner`
#[derive(Debug)]
pub enum ScalarMiningMode {
    /// A miner that does nothing
    None,
    /// A miner that listens for new transactions that are ready.
    ///
    /// Either one transaction will be mined per block, or any number of transactions will be
    /// allowed
    Auto(ReadyTransactionMiner),
    /// A miner that listens commited transactions from external Narwhal consensus
    Narwhal(NarwhalTransactionMiner),
    /// A miner that constructs a new block every `interval` tick
    FixedBlockTime(FixedBlockTimeMiner),
}

// === impl MiningMode ===

impl ScalarMiningMode {
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
        rx_pending_trans: Receiver<TxHash>,
        rx_commited_trans: UnboundedReceiver<Vec<ExternalTransaction>>,
    ) -> Self {
        ScalarMiningMode::Narwhal(NarwhalTransactionMiner {
            max_transactions: usize::MAX,
            has_pending_txs: None,
            rx_pending_trans: ReceiverStream::new(rx_pending_trans).fuse(),
            rx_commited_trans: UnboundedReceiverStream::new(rx_commited_trans).fuse(),
            commited_transactions: HashSet::default(),
            queue_pending_txs: Default::default(),
        })
    }

    /// Creates a new interval miner that builds a block ever `duration`.
    pub fn interval(duration: Duration) -> Self {
        ScalarMiningMode::FixedBlockTime(FixedBlockTimeMiner::new(duration))
    }

    /// polls the Pool and returns those transactions that should be put in a block, if any.
    pub(crate) fn poll<Pool>(
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
pub struct NarwhalTransactionMiner {
    /// how many transactions to mine per block
    max_transactions: usize,
    /// stores whether there are pending transactions (if known)
    has_pending_txs: Option<bool>,
    /// Receives hashes of transactions that are ready
    rx_pending_trans: Fuse<ReceiverStream<TxHash>>,
    rx_commited_trans: Fuse<UnboundedReceiverStream<Vec<ExternalTransaction>>>,
    commited_transactions: HashSet<Vec<u8>>,
    /// Store pending transactions in original order
    queue_pending_txs: VecDeque<Vec<u8>>,
}

// === impl NarwhalTransactionMiner ===

impl NarwhalTransactionMiner {
    fn poll<Pool>(
        &mut self,
        pool: &Pool,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>>
    where
        Pool: TransactionPool,
    {
        while let Poll::Ready(Some(transactions)) =
            Pin::new(&mut self.rx_commited_trans).poll_next(cx)
        {
            for ExternalTransaction { namespace, tx_bytes } in transactions.into_iter() {
                self.commited_transactions.insert(tx_bytes);
            }
        }
        // drain the notification stream
        while let Poll::Ready(Some(hash)) = Pin::new(&mut self.rx_pending_trans).poll_next(cx) {
            self.queue_pending_txs.push_back(hash.as_slice().to_vec());
            //self.has_pending_txs = Some(true);
        }

        info!(
            "pending queue {:?}, commited transaction count {:?}",
            self.queue_pending_txs.len(),
            self.commited_transactions.len()
        );
        if self.queue_pending_txs.len() > self.commited_transactions.len() {
            return Poll::Pending;
        }
        info!("Commited transactions length {:?}", self.commited_transactions.len());
        let mut tx_counter = 0;

        while let Some(tx_hash) = self.queue_pending_txs.get(0) {
            if self.commited_transactions.remove(tx_hash) {
                tx_counter += 1;
                self.queue_pending_txs.pop_front();
            } else {
                break;
            }
        }
        if tx_counter == 0 {
            return Poll::Pending;
        }
        info!("tx_counter {:?}", tx_counter);
        let transactions = pool.best_transactions().take(tx_counter).collect::<Vec<_>>();
        info!("Found {:?} transactions for the next block", transactions.len());
        if transactions.is_empty() {
            return Poll::Pending;
        }

        Poll::Ready(transactions)
    }
}

impl fmt::Debug for NarwhalTransactionMiner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NarwhalTransactionMiner")
            .field("max_transactions", &self.max_transactions)
            .finish_non_exhaustive()
    }
}
