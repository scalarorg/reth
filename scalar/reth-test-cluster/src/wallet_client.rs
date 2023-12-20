use std::option;

use ethers::{
    core::k256::ecdsa::SigningKey, prelude::*, signers::coins_bip39::English,
    types::transaction::eip2718::TypedTransaction,
};

use crate::{cluster::Cluster, config::ClusterTestOpt};
pub struct WalletClient {
    wallet_context: Wallet<SigningKey>,
    address: String,
    provider: Provider<Http>,
    index: u32,
}

impl WalletClient {
    pub async fn new_from_cluster(
        cluster: &(dyn Cluster + Sync + Send),
        options: &ClusterTestOpt,
    ) -> Self {
        let provider =
            Provider::<Http>::try_from(cluster.fullnode_url()).expect("Should create provider");
        let chain_id = provider.get_chainid().await.expect("Should get chain id").as_u64();
        let phrase: &str = options.phrase();
        // Use instance for index or 0 for first account
        let index: u32 = options.instance().unwrap_or(0) as u32;

        // instantiate the wallet with the phrase and the index of the account we want to use
        let wallet_context = MnemonicBuilder::<English>::default()
            .phrase(phrase)
            .index(index)
            .expect("Should be able to create mnemonic")
            .build()
            .expect("Should be able to build mnemonic")
            .with_chain_id(chain_id);

        let address = wallet_context.address();

        let address = format!("{address:#20x}");

        Self { wallet_context, address, provider, index }
    }

    pub fn get_wallet(&self) -> &Wallet<SigningKey> {
        &self.wallet_context
    }

    pub fn get_wallet_mut(&mut self) -> &mut Wallet<SigningKey> {
        &mut self.wallet_context
    }

    pub fn get_wallet_address(&self) -> &str {
        &self.address
    }

    pub fn get_fullnode_client(&self) -> &Provider<Http> {
        &self.provider
    }

    pub fn get_index(&self) -> u32 {
        self.index
    }

    pub async fn sign(&self, tx: &TypedTransaction) -> Result<Signature, WalletError> {
        self.wallet_context.sign_transaction(tx).await
    }

    pub fn create_transaction(&self, to: &str, value: U256, nonce: u64) -> TransactionRequest {
        let chain_id = self.wallet_context.chain_id();
        TransactionRequest::new()
            .to(to)
            .nonce(nonce)
            .value(value)
            .gas(100000)
            .gas_price(0x43423422)
            .chain_id(chain_id)
    }
}
