use anyhow::Ok;
use ethers::{providers::Middleware, types::BlockId, types::BlockNumber};
use tracing::{error, info};

use crate::{config::ClusterTestOpt, wallet_client::WalletClient};

pub async fn send_raw_transactions(
    clients: &Vec<WalletClient>,
    options: &ClusterTestOpt,
    tx_amount: u64,
) -> Result<(), anyhow::Error> {
    let mut transaction_count_clusters = vec![];

    // Get the transaction count for each client
    for client in clients {
        let transaction_count = client
            .get_fullnode_client()
            .get_transaction_count(
                client.get_wallet_address(),
                Some(BlockId::Number(BlockNumber::Pending)),
            )
            .await
            .expect("Should get transaction count")
            .as_u64();

        transaction_count_clusters.push(transaction_count);
    }

    info!("Transaction count for each cluster: {:?}", transaction_count_clusters);

    // Send the transactions
    for transaction_index in 0..tx_amount {
        for (client_index, client) in clients.iter().enumerate() {
            let nonce = transaction_count_clusters[client_index] + transaction_index;
            if let Err(err) = send_raw_transaction(client, options, nonce).await {
                error!("Error sending transaction: {:?}", err);
            }
        }
    }

    Ok(())
}

async fn send_raw_transaction(
    client: &WalletClient,
    options: &ClusterTestOpt,
    nonce: u64,
) -> Result<(), anyhow::Error> {
    let receiver_address = options.receiver_address();

    info!("Instance {:?}: Sending transaction with nonce {}", client.get_index(), nonce);
    let tx = client.create_transaction(receiver_address, 100000u64.into(), nonce).into();

    let signature = client.sign(&tx).await?;
    let raw_tx = tx.rlp_signed(&signature);

    let result = client.get_fullnode_client().send_raw_transaction(raw_tx).await?;

    assert!(result.tx_hash().to_string().starts_with("0x"), "invalid tx hash");

    Ok(())
}
