use clap::{Parser, Subcommand};
use reth_test_cluster::{config::ClusterTestOpt, utils::transaction::send_raw_transactions};
use tracing::info;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
#[clap(rename_all = "snake_case")]
enum Command {
    SendRawTx { amount: u64 },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let _guard = telemetry_subscribers::TelemetryConfig::new().with_env().init();
    let mut options = ClusterTestOpt::parse();

    match args.command {
        Command::SendRawTx { amount } => {
            info!("Sending {} transactions to each cluster", amount);

            let nodes = options.nodes();

            let mut clients = vec![];

            for instance in 1..=nodes {
                options.set_instance(instance);
                // HTTP_RPC_PORT: default - `instance` + 1
                let cluster_port = 8545 - instance as u32 + 1;
                let wallet_client =
                    reth_test_cluster::wallet_client::WalletClient::new_from_cluster_url(
                        &format!("http://localhost:{}", cluster_port),
                        &options,
                    )
                    .await;
                clients.push(wallet_client);
            }

            send_raw_transactions(&clients, &options, amount).await?;
        }

        _ => panic!("Invalid command"),
    }

    Ok(())
}
