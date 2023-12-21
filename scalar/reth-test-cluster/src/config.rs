use dotenvy::dotenv;
use serde::Deserialize;
use tracing::info;

#[derive(derivative::Derivative, Deserialize)]
#[derivative(Debug)]
pub struct ClusterTestOpt {
    pub nodes: u8,
    pub chain: String,
    pub phrase: String,
    pub receiver_address: String,
    pub transaction_amount: u64,
    pub narwhal_port: Option<String>,
    pub instance: Option<u8>,
    pub wait_time_ms: Option<u64>,
    pub keep_alive: Option<bool>,
    pub txpool_pending_max_count: Option<usize>,
    pub txpool_pending_max_size: Option<usize>,
    pub txpool_basefee_max_count: Option<usize>,
    pub txpool_basefee_max_size: Option<usize>,
    pub txpool_queued_max_count: Option<usize>,
    pub txpool_queued_max_size: Option<usize>,
    pub txpool_max_account_slots: Option<usize>,
    pub txpool_price_bump: Option<u128>,
    pub txpool_blob_transaction_price_bump: Option<u128>,
    pub txpool_no_locals: Option<bool>,
}

impl ClusterTestOpt {
    pub fn parse() -> Self {
        // Load .env file if it exists
        if dotenv().is_err() {
            info!("No .env file found, using environment variables");
        }

        match envy::from_env::<Self>() {
            Ok(config) => config,
            Err(e) => panic!("Couldn't read config ({})", e),
        }
    }

    pub fn phrase(&self) -> &str {
        &self.phrase
    }

    pub fn receiver_address(&self) -> &str {
        &self.receiver_address
    }

    pub fn nodes(&self) -> u8 {
        self.nodes
    }

    pub fn chain(&self) -> &str {
        &self.chain
    }

    pub fn transaction_amount(&self) -> u64 {
        self.transaction_amount
    }

    pub fn narwhal_port(&self) -> &Option<String> {
        &self.narwhal_port
    }

    pub fn instance(&self) -> &Option<u8> {
        &self.instance
    }

    pub fn set_instance(&mut self, instance: u8) {
        self.instance = Some(instance);
    }

    pub fn wait_time_ms(&self) -> u64 {
        if self.wait_time_ms.is_none() {
            return 20000;
        }

        self.wait_time_ms.unwrap()
    }

    pub fn keep_alive(&self) -> bool {
        if self.keep_alive.is_none() {
            return false;
        }

        self.keep_alive.unwrap()
    }

    pub fn txpool_pending_max_count(&self) -> Option<usize> {
        self.txpool_pending_max_count
    }

    pub fn txpool_pending_max_size(&self) -> Option<usize> {
        self.txpool_pending_max_size
    }

    pub fn txpool_basefee_max_count(&self) -> Option<usize> {
        self.txpool_basefee_max_count
    }

    pub fn txpool_basefee_max_size(&self) -> Option<usize> {
        self.txpool_basefee_max_size
    }

    pub fn txpool_queued_max_count(&self) -> Option<usize> {
        self.txpool_queued_max_count
    }

    pub fn txpool_queued_max_size(&self) -> Option<usize> {
        self.txpool_queued_max_size
    }

    pub fn txpool_max_account_slots(&self) -> Option<usize> {
        self.txpool_max_account_slots
    }

    pub fn txpool_price_bump(&self) -> Option<u128> {
        self.txpool_price_bump
    }

    pub fn txpool_blob_transaction_price_bump(&self) -> Option<u128> {
        self.txpool_blob_transaction_price_bump
    }

    pub fn txpool_no_locals(&self) -> Option<bool> {
        self.txpool_no_locals
    }
}
