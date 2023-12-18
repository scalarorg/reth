use crate::reth_cluster::{TestCluster, TestClusterBuilder};
use anyhow::Ok;
use async_trait::async_trait;
use tracing::info;

use crate::config::ClusterTestOpt;

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
    handles: Vec<tokio::task::JoinHandle<()>>,
    tx_shutdowns: Vec<tokio::sync::watch::Sender<()>>,
}

#[async_trait]
impl Cluster for LocalNewCluster {
    async fn start(options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        let mut cluster_builder = TestClusterBuilder::new();
        let mut handles = vec![];
        let mut tx_shutdowns = vec![];

        for instance in 1..=options.nodes {
            cluster_builder
                .narwhal_port(options.narwhal_port.clone())
                .chain(options.chain.clone())
                .instance(instance);
            let mut test_cluster = cluster_builder.build();

            let (tx_shutdown, mut rx_shutdown) = tokio::sync::watch::channel::<()>(());

            *rx_shutdown.borrow_and_update();

            let handle = tokio::task::spawn_blocking(move || {
                test_cluster.start(rx_shutdown).expect("Failed to start cluster");
            });

            handles.push(handle);
            tx_shutdowns.push(tx_shutdown);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        let test_cluster = cluster_builder.chain(options.chain.clone()).build();

        let fullnode_url = test_cluster.fullnode_url().to_string();

        Ok(Self { fullnode_url, test_cluster, handles, tx_shutdowns })
    }

    fn fullnode_url(&self) -> &str {
        &self.fullnode_url
    }

    async fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        for tx in self.tx_shutdowns.drain(..) {
            tx.send(()).expect("Should send shutdown signal to cluster");
        }

        info!("Waiting for cluster to shutdown...");
        // TODO: Improve the shutdown process
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        for handle in &self.handles {
            handle.abort();
        }

        Ok(())
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
