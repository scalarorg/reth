use async_trait::async_trait;
use ethers::providers::Middleware;
use tracing::info;

use crate::{TestCaseImpl, TestContext};

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
        let sender_address = ctx.client.get_wallet_address();
        let receiver_address = ctx.options.receiver_address();
        let transaction_count = ctx
            .client
            .get_fullnode_client()
            .get_transaction_count(sender_address, None)
            .await?
            .as_u64();

        let transaction_amount = ctx.options.transaction_amount();

        let max_nonce = transaction_count + transaction_amount;

        info!(
            "Sending {} transactions from {} to {}",
            transaction_amount, sender_address, receiver_address
        );

        for nonce in transaction_count..max_nonce {
            let tx = ctx.client.create_transaction(sender_address, 100000u64.into(), nonce).into();

            let signature = ctx.client.sign(&tx).await?;
            let raw_tx = tx.rlp_signed(&signature);

            let result = ctx.client.get_fullnode_client().send_raw_transaction(raw_tx).await?;

            assert!(result.tx_hash().to_string().starts_with("0x"), "invalid tx hash");
        }

        // Sleep for 10 seconds to allow the transactions to be processed
        // TODO: Use a better way to check if the transactions are processed
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        Ok(())
    }
}
