use crate::config::ClusterTestOpt;
use crate::reth_cluster::{TestCluster, TestClusterBuilder};
use anyhow::Ok;
use async_trait::async_trait;

pub struct ClusterFactory;

impl ClusterFactory {
    pub async fn start(
        options: &ClusterTestOpt,
    ) -> Result<Box<dyn Cluster + Sync + Send>, anyhow::Error> {
        // Ok(match &options.env {
        //     Env::NewLocal => Box::new(LocalNewCluster::start(options).await?),
        //     _ => Box::new(RemoteRunningCluster::start(options).await?),
        // })
        Ok(Box::new(LocalNewCluster::start(options).await?))
    }
}

/// Cluster Abstraction
#[async_trait]
pub trait Cluster {
    async fn start(options: &ClusterTestOpt) -> Result<Self, anyhow::Error>
    where
        Self: Sized;

    fn fullnode_url(&self) -> &str;

    async fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn get_cluster_handle(&mut self) -> Result<tokio::task::JoinHandle<()>, anyhow::Error> {
        Ok(tokio::task::spawn(async move {}))
    }
}

/// Represents an up and running cluster deployed remotely.
pub struct RemoteRunningCluster {
    fullnode_url: String,
    config_directory: tempfile::TempDir,
}

#[async_trait]
impl Cluster for RemoteRunningCluster {
    async fn start(_options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        // TODO: Deploy a cluster to a remote location.
        let fullnode_url = "http://localhost:8545".to_string();

        Ok(Self { config_directory: tempfile::tempdir()?, fullnode_url })
    }

    fn fullnode_url(&self) -> &str {
        &self.fullnode_url
    }
}

/// Represents a local Cluster which starts per cluster test run.
pub struct LocalNewCluster {
    test_cluster: TestCluster,
    fullnode_url: String,
    handle: Option<tokio::task::JoinHandle<()>>,
    tx_shutdown: tokio::sync::watch::Sender<()>,
}

#[async_trait]
impl Cluster for LocalNewCluster {
    async fn start(options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        let mut cluster_builder = TestClusterBuilder::new();

        cluster_builder
            .narwhal_port(options.narwhal_port.clone())
            .chain(options.chain.clone())
            .pending_max_count(options.txpool_pending_max_count)
            .pending_max_size(options.txpool_pending_max_size)
            .basefee_max_count(options.txpool_basefee_max_count)
            .basefee_max_size(options.txpool_basefee_max_size)
            .queued_max_count(options.txpool_queued_max_count)
            .queued_max_size(options.txpool_queued_max_size)
            .max_account_slots(options.txpool_max_account_slots)
            .price_bump(options.txpool_price_bump())
            .blob_transaction_price_bump(options.txpool_blob_transaction_price_bump())
            .no_locals(options.txpool_no_locals);

        if options.instance.is_some() {
            cluster_builder.instance(options.instance.unwrap());
        }

        let mut test_cluster = cluster_builder.build();

        let (tx_shutdown, mut rx_shutdown) = tokio::sync::watch::channel::<()>(());

        // Get the initial value from the watch channel
        #[allow(clippy::unnecessary_operation)]
        *rx_shutdown.borrow_and_update();

        let handle = tokio::task::spawn_blocking(move || {
            test_cluster.start(rx_shutdown).expect("Failed to start cluster");
        });

        let test_cluster = cluster_builder.chain(options.chain.clone()).build();

        let fullnode_url = test_cluster.fullnode_url().to_string();

        Ok(Self { fullnode_url, test_cluster, handle: Some(handle), tx_shutdown })
    }

    fn fullnode_url(&self) -> &str {
        &self.fullnode_url
    }

    async fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        self.tx_shutdown.send(()).expect("Should send shutdown signal to cluster");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }

        Ok(())
    }

    fn get_cluster_handle(&mut self) -> Result<tokio::task::JoinHandle<()>, anyhow::Error> {
        if let Some(handle) = self.handle.take() {
            return Ok(handle);
        }

        Err(anyhow::anyhow!("Cluster handle not found"))
    }
}

// Make linter happy
#[async_trait]
impl Cluster for Box<dyn Cluster + Send + Sync> {
    async fn start(_options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        unreachable!(
            "If we already have a boxed Cluster trait object we wouldn't have to call this function"
        );
    }
    fn fullnode_url(&self) -> &str {
        (**self).fullnode_url()
    }

    async fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        (**self).shutdown().await
    }
}
