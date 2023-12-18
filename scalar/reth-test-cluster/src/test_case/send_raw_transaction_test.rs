use async_trait::async_trait;
use ethers::providers::Middleware;
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
        for client in &ctx.clients {
            send_transaction_raw(client, &ctx.options).await;
        }

        // Sleep for 10 seconds to allow the transactions to be processed
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        Ok(())
    }
}

async fn send_transaction_raw(client: &WalletClient, options: &ClusterTestOpt) {
    let sender_address = client.get_wallet_address();
    let receiver_address = options.receiver_address();
    let transaction_count = client
        .get_fullnode_client()
        .get_transaction_count(sender_address, None)
        .await
        .expect("Should get transaction count")
        .as_u64();

    let transaction_amount = options.transaction_amount();

    let max_nonce = transaction_count + transaction_amount;

    info!(
        "Sending {} transactions from {} to {}",
        transaction_amount, sender_address, receiver_address
    );

    for nonce in transaction_count..max_nonce {
        info!("{:?}: Sending transaction with nonce {}", options.instance, nonce);
        let tx = client.create_transaction(sender_address, 100000u64.into(), nonce).into();

        let signature = client.sign(&tx).await.expect("Should sign transaction");
        let raw_tx = tx.rlp_signed(&signature);

        let result = client
            .get_fullnode_client()
            .send_raw_transaction(raw_tx)
            .await
            .expect("Should send raw transaction");

        assert!(result.tx_hash().to_string().starts_with("0x"), "invalid tx hash");
    }
}
