use async_trait::async_trait;
use cluster::{Cluster, ClusterFactory};
use config::ClusterTestOpt;
use test_case::send_raw_transaction_test::SendRawTransactionTest;
use tracing::{error, info};
use wallet_client::WalletClient;

pub mod cluster;
pub mod config;
pub mod reth_cluster;
pub mod test_case;
pub mod wallet_client;

#[allow(unused)]
pub struct TestContext {
    /// Cluster handle that allows access to various components in a cluster
    cluster: Box<dyn Cluster + Sync + Send>,
    /// Client that provides wallet context and fullnode access
    client: WalletClient,
    options: ClusterTestOpt,
}

impl TestContext {
    pub async fn setup(options: ClusterTestOpt) -> Result<Self, anyhow::Error> {
        let cluster = ClusterFactory::start(&options).await?;

        // Sleep for a bit to allow the cluster to start up
        // TODO: Use a better way to check if the cluster is up and running
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        let wallet_client = WalletClient::new_from_cluster(&cluster, &options).await;
        Ok(Self { cluster, client: wallet_client, options })
    }
}

pub struct ClusterTest;

impl ClusterTest {
    pub async fn run(options: ClusterTestOpt) {
        let mut ctx = TestContext::setup(options)
            .await
            .unwrap_or_else(|e| panic!("Failed to setup test context: {e}"));

        let tests = vec![TestCase::new(SendRawTransactionTest {})];

        // TODO: improve the runner parallelism for efficiency
        // For now we run tests serially
        let mut success_cnt = 0;
        let total_cnt = tests.len() as i32;
        for t in tests {
            let is_success = t.run(&mut ctx).await as i32;
            success_cnt += is_success;
        }
        if success_cnt < total_cnt {
            // If any test failed, panic to bubble up the signal
            panic!("{success_cnt} of {total_cnt} tests passed.");
        }
        info!("{success_cnt} of {total_cnt} tests passed.");

        ctx.cluster.shutdown().await.unwrap_or_else(|e| {
            error!("Failed to shutdown cluster: {e}");
        });
    }
}

#[async_trait]
pub trait TestCaseImpl {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    async fn run(&self, ctx: &mut TestContext) -> Result<(), anyhow::Error>;
}

pub struct TestCase<'a> {
    test_case: Box<dyn TestCaseImpl + 'a>,
}

impl<'a> TestCase<'a> {
    pub fn new(test_case: impl TestCaseImpl + 'a) -> Self {
        TestCase { test_case: (Box::new(test_case)) }
    }

    pub async fn run(self, ctx: &mut TestContext) -> bool {
        let test_name = self.test_case.name();
        info!("Running test {}.", test_name);

        // TODO: unwind panic and fail gracefully?

        match self.test_case.run(ctx).await {
            Ok(()) => {
                info!("Test {test_name} succeeded.");
                true
            }
            Err(e) => {
                error!("Test {test_name} failed with error: {e}.");
                false
            }
        }
    }
}
