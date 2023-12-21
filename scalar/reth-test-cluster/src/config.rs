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
}
