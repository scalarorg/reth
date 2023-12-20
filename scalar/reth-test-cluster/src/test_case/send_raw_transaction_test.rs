use async_trait::async_trait;
use ethers::{providers::Middleware, types::transaction};
use tracing::info;

use crate::{config::ClusterTestOpt, wallet_client::WalletClient, TestCaseImpl, TestContext};

pub struct SendRawTransactionTest;

#[async_trait]
impl TestCaseImpl for SendRawTransactionTest {
    fn name(&self) -> &'static str {
        "SendRawTransaction"
    }

    fn description(&self) -> &'static str {
        "Test sending a raw transaction to the cluster."
    }

    async fn run(&self, ctx: &mut TestContext) -> Result<(), anyhow::Error> {
        let transaction_amount = ctx.options.transaction_amount();
        let mut transaction_count_clusters = vec![];

        // Get the transaction count for each client
        for client in &ctx.clients {
            let transaction_count = client
                .get_fullnode_client()
                .get_transaction_count(client.get_wallet_address(), None)
                .await
                .expect("Should get transaction count")
                .as_u64();

            transaction_count_clusters.push(transaction_count);
        }

        // Send the transactions
        for transaction_index in 0..transaction_amount {
            for (client_index, client) in ctx.clients.iter().enumerate() {
                let nonce = transaction_count_clusters[client_index] + transaction_index;
                send_transaction_raw(client, &ctx.options, nonce).await;
            }
        }

        // Sleep for X milliseconds to allow the transactions to be processed
        tokio::time::sleep(std::time::Duration::from_millis(ctx.options.wait_time_ms())).await;

        Ok(())
    }
}

async fn send_transaction_raw(client: &WalletClient, options: &ClusterTestOpt, nonce: u64) {
    let receiver_address = options.receiver_address();

    info!("Instance {:?}: Sending transaction with nonce {}", client.get_index(), nonce);
    let tx = client.create_transaction(receiver_address, 100000u64.into(), nonce).into();

    let signature = client.sign(&tx).await.expect("Should sign transaction");
    let raw_tx = tx.rlp_signed(&signature);

    let result = client
        .get_fullnode_client()
        .send_raw_transaction(raw_tx)
        .await
        .expect("Should send raw transaction");

    assert!(result.tx_hash().to_string().starts_with("0x"), "invalid tx hash");
}
