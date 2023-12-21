use crate::{utils::transaction::send_raw_transactions, TestCaseImpl, TestContext};
use async_trait::async_trait;

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
        let tx_amount = ctx.options.transaction_amount();
        send_raw_transactions(&ctx.clients, &ctx.options, tx_amount).await?;

        // Sleep for X milliseconds to allow the transactions to be processed
        tokio::time::sleep(std::time::Duration::from_millis(ctx.options.wait_time_ms())).await;

        Ok(())
    }
}
