use reth_test_cluster::{config::ClusterTestOpt, ClusterTest};

#[tokio::main]
async fn main() {
    let _guard = telemetry_subscribers::TelemetryConfig::new().with_env().init();

    let options = ClusterTestOpt::parse();

    ClusterTest::run(options).await;
}
