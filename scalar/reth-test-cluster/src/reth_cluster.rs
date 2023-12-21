use clap::Parser;
use futures::pin_mut;
use reth_tasks::TaskManager;
use scalar_reth::node::NodeCommand;
use scalar_reth::runner::{tokio_runtime, CliContext};
use scalar_reth::transaction_pool::{
    DEFAULT_PRICE_BUMP, REPLACE_BLOB_PRICE_BUMP, TXPOOL_MAX_ACCOUNT_SLOTS_PER_SENDER,
    TXPOOL_SUBPOOL_MAX_SIZE_MB_DEFAULT, TXPOOL_SUBPOOL_MAX_TXS_DEFAULT,
};
use std::future::Future;
use tokio::sync::watch;
use tracing::{info, trace};

#[derive(Clone, Default)]
pub struct TestCluster {
    metrics_port: String,
    metrics_addr: String,
    data_dir: String,
    ws_port: String,
    ws_addr: String,
    http_port: String,
    http_addr: String,
    auth_port: String,
    auth_addr: String,
    auth_jwt_secret: String,
    instance: u8,
    chain: String,
    narwhal_port: Option<String>,

    pending_max_count: usize,
    pending_max_size: usize,
    basefee_max_count: usize,
    basefee_max_size: usize,
    queued_max_count: usize,
    queued_max_size: usize,
    max_account_slots: usize,
    price_bump: u128,
    blob_transaction_price_bump: u128,
    no_locals: bool,
}

// Metrics
const DEFAULT_METRICS_ADDR: &str = "127.0.0.1";
const DEFAULT_METRICS_PORT: u32 = 9001;
const DEFAULT_DATA_DIR: &str = "$HOME/.local/share/reth";
const DEFAULT_WS_ADDR: &str = "0.0.0.0";
const DEFAULT_WS_PORT: u32 = 8546;
const DEFAULT_HTTP_ADDR: &str = "0.0.0.0";
const DEFAULT_HTTP_PORT: u32 = 8545;

// Auth
const DEFAULT_AUTH_ADDR: &str = "127.0.0.1";
const DEFAULT_AUTH_PORT: u32 = 8551;
const DEFAULT_AUTH_JWT_SECRET: &str = "$HOME/.local/share/reth/sepolia/jwt.hex";

// Instance
const DEFAULT_INSTANCE: u8 = 1;

// Chain
const DEFAULT_CHAIN: &str = "sepolia";

impl TestCluster {
    pub fn start(&mut self, rx_shutdown: watch::Receiver<()>) -> eyre::Result<()> {
        // let narwhal = if self.narwhal_port.is_some() {
        //     "--narwhal"
        // } else {
        //     // TODO: Chose a better way to turn off narwhal
        //     "--ws"
        // };

        let no_local = if self.no_locals { "--txpool.nolocals" } else { "--" };

        let node_cmd = NodeCommand::<()>::try_parse_from([
            "reth node",
            "--chain",
            self.chain.as_str(),
            "--http",
            "--http.addr",
            self.http_addr.as_str(),
            "--http.port",
            self.http_port.as_str(),
            "--http.api",
            "eth,net,trace,web3,rpc,debug,txpool",
            "--http.corsdomain",
            "*",
            "--ws",
            "--ws.addr",
            self.ws_addr.as_str(),
            "--ws.port",
            self.ws_port.as_str(),
            "--ws.api",
            "eth,net,trace,web3,rpc,debug,txpool",
            "--metrics",
            format!("{}:{}", self.metrics_addr, self.metrics_port).as_str(),
            "--datadir",
            self.data_dir.as_str(),
            // "--authrpc.port",
            // self.auth_port.as_str(),
            // "--authrpc.addr",
            // self.auth_addr.as_str(),
            // "--authrpc.jwtsecret",
            // self.auth_jwt_secret.as_str(),
            "--instance",
            self.instance.to_string().as_str(),
            "--narwhal",
            "--narwhal.port",
            self.narwhal_port.as_ref().unwrap_or(&"9090".to_string()).as_str(),
            // SCALAR-TODO: Find a way to get the bootnodes from local
            "--bootnodes",
            "enode://4901ba0085be8a1fb1e7725c4c6582668f5cd79e1da4a459207abf6afded09e41d23561d6a239183825bf14e80900c46f4c18d338095db79b4f2490c8fef81be@127.0.0.1:30303,enode://354f037e3fb7c2df27eb9de1f20f4a2f539e97379afd65de96c182637a86a1ae6f7bf8ffcf87eb3233839874449b3122f745139a85d055898bf7beef81f66d17@127.0.0.1:30304,enode://129a37d3192d394503ca3ca5ebeef2c6f63f04ad968cc79c222423aea1c3db633b5bf2d5538f39c8c1a103b2277c28df583d6c417dc928bf3fe7ecbb4e035ddc@127.0.0.1:30305,enode://17910578c735e05e3faee82e4890eaba8fac4cd6f6b94bd84c8c89bd6265024ff5f41aff1326d5f7b921092223385e937eb34fd101603eccfd88228433c2ccab@127.0.0.1:30306",
            "--txpool.pending_max_count",
            self.pending_max_count.to_string().as_str(),
            "--txpool.pending_max_size",
            self.pending_max_size.to_string().as_str(),
            "--txpool.basefee_max_count",
            self.basefee_max_count.to_string().as_str(),
            "--txpool.basefee_max_size",
            self.basefee_max_size.to_string().as_str(),
            "--txpool.queued_max_count",
            self.queued_max_count.to_string().as_str(),
            "--txpool.queued_max_size",
            self.queued_max_size.to_string().as_str(),
            "--txpool.max_account_slots",
            self.max_account_slots.to_string().as_str(),
            "--txpool.pricebump",
            self.price_bump.to_string().as_str(),
            "--blobpool.pricebump",
            self.blob_transaction_price_bump.to_string().as_str(),
            // "--txpool.nolocals",
            no_local,
        ])
        .expect("Parse node command");

        let tokio_runtime = tokio_runtime()?;
        let task_manager = TaskManager::new(tokio_runtime.handle().clone());
        let task_executor = task_manager.executor();
        let context = CliContext { task_executor };

        let task_manager = tokio_runtime.block_on(run_to_completion_or_panic(
            task_manager,
            run_until_ctrl_c(node_cmd.execute(context), rx_shutdown.clone()),
            rx_shutdown,
        ))?;

        // after the command has finished or exit signal was received we drop the task manager which
        // fires the shutdown signal to all tasks spawned via the task executor
        drop(task_manager);

        // drop the tokio runtime on a separate thread because drop blocks until its pools
        // (including blocking pool) are shutdown. In other words `drop(tokio_runtime)` would block
        // the current thread but we want to exit right away.
        std::thread::spawn(move || drop(tokio_runtime));

        // give all tasks that are now being shut down some time to finish before tokio leaks them
        // see [Runtime::shutdown_timeout](tokio::runtime::Runtime::shutdown_timeout)
        // TODO: enable this again, when pipeline/stages are not longer blocking tasks
        // warn!(target: "reth::cli", "Received shutdown signal, waiting up to 30 seconds for
        // tasks."); tokio_runtime.shutdown_timeout(Duration::from_secs(30));
        Ok(())
    }

    pub fn fullnode_url(&self) -> String {
        format!(
            "http://{}:{}",
            self.http_addr,
            // http port base on instance number, see detail here: https://paradigmxyz.github.io/reth/cli/node.html
            self.http_port.parse::<u32>().expect("http port should be a number")
                - self.instance as u32
                + 1
        )
    }
}

pub struct TestClusterBuilder {
    metrics_port: String,
    metrics_addr: String,
    ws_port: String,
    ws_addr: String,
    http_port: String,
    http_addr: String,
    data_dir: String,
    auth_port: String,
    auth_addr: String,
    auth_jwt_secret: String,
    instance: u8,
    chain: String,
    narwhal_port: Option<String>,
    pending_max_count: usize,
    pending_max_size: usize,
    basefee_max_count: usize,
    basefee_max_size: usize,
    queued_max_count: usize,
    queued_max_size: usize,
    max_account_slots: usize,
    price_bump: u128,
    blob_transaction_price_bump: u128,
    no_locals: bool,
}

impl Default for TestClusterBuilder {
    fn default() -> Self {
        Self {
            metrics_port: DEFAULT_METRICS_PORT.to_string(),
            metrics_addr: DEFAULT_METRICS_ADDR.to_string(),
            data_dir: DEFAULT_DATA_DIR.to_string(),
            ws_port: DEFAULT_WS_PORT.to_string(),
            ws_addr: DEFAULT_WS_ADDR.to_string(),
            http_port: DEFAULT_HTTP_PORT.to_string(),
            http_addr: DEFAULT_HTTP_ADDR.to_string(),
            auth_port: DEFAULT_AUTH_PORT.to_string(),
            auth_addr: DEFAULT_AUTH_ADDR.to_string(),
            auth_jwt_secret: DEFAULT_AUTH_JWT_SECRET.to_string(),
            instance: DEFAULT_INSTANCE,
            chain: DEFAULT_CHAIN.to_string(),
            narwhal_port: None,
            pending_max_count: TXPOOL_SUBPOOL_MAX_TXS_DEFAULT,
            pending_max_size: TXPOOL_SUBPOOL_MAX_SIZE_MB_DEFAULT,
            basefee_max_count: TXPOOL_SUBPOOL_MAX_TXS_DEFAULT,
            basefee_max_size: TXPOOL_SUBPOOL_MAX_SIZE_MB_DEFAULT,
            queued_max_count: TXPOOL_SUBPOOL_MAX_TXS_DEFAULT,
            queued_max_size: TXPOOL_SUBPOOL_MAX_SIZE_MB_DEFAULT,
            max_account_slots: TXPOOL_MAX_ACCOUNT_SLOTS_PER_SENDER,
            price_bump: DEFAULT_PRICE_BUMP,
            blob_transaction_price_bump: REPLACE_BLOB_PRICE_BUMP,
            no_locals: false,
        }
    }
}

impl TestClusterBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn metrics_port(&mut self, metrics_port: String) -> &mut Self {
        self.metrics_port = metrics_port;
        self
    }

    pub fn metrics_addr(&mut self, metrics_addr: String) -> &mut Self {
        self.metrics_addr = metrics_addr;
        self
    }

    pub fn data_dir(&mut self, data_dir: String) -> &mut Self {
        self.data_dir = data_dir;
        self
    }

    pub fn ws_port(&mut self, ws_port: String) -> &mut Self {
        self.ws_port = ws_port;
        self
    }

    pub fn http_port(&mut self, http_port: String) -> &mut Self {
        self.http_port = http_port;
        self
    }

    pub fn http_addr(&mut self, http_addr: String) -> &mut Self {
        self.http_addr = http_addr;
        self
    }

    pub fn auth_port(&mut self, auth_port: String) -> &mut Self {
        self.auth_port = auth_port;
        self
    }

    pub fn auth_addr(&mut self, auth_addr: String) -> &mut Self {
        self.auth_addr = auth_addr;
        self
    }

    pub fn auth_jwt_secret(&mut self, auth_jwt_secret: String) -> &mut Self {
        self.auth_jwt_secret = auth_jwt_secret;
        self
    }

    /// Sets the instance number of the node. This is used to calculate the port of the node.
    ///
    /// Should be called right before `build`.
    pub fn instance(&mut self, instance: u8) -> &mut Self {
        self.instance = instance;
        self.data_dir = format!("{}/{}", DEFAULT_DATA_DIR, instance);
        if self.narwhal_port.is_some() {
            self.narwhal_port = Some(
                (self
                    .narwhal_port
                    .as_ref()
                    .unwrap()
                    .parse::<u32>()
                    .expect("narwhal port should be a number")
                    + instance as u32
                    - 1)
                .to_string(),
            );
        }
        // self.metrics_port =
        //     (self.metrics_port.parse::<u32>().expect("metrics port should be a number")
        //         + instance as u32
        //         - 1)
        //     .to_string();
        self
    }

    pub fn chain(&mut self, chain: String) -> &mut Self {
        self.chain = chain;
        self
    }

    pub fn narwhal_port(&mut self, narwhal_port: Option<String>) -> &mut Self {
        self.narwhal_port = narwhal_port;
        self
    }

    pub fn pending_max_count(&mut self, pending_max_count: Option<usize>) -> &mut Self {
        if let Some(pending_max_count) = pending_max_count {
            self.pending_max_count = pending_max_count;
        }
        self
    }

    pub fn pending_max_size(&mut self, pending_max_size: Option<usize>) -> &mut Self {
        if let Some(pending_max_size) = pending_max_size {
            self.pending_max_size = pending_max_size;
        }
        self
    }

    pub fn basefee_max_count(&mut self, basefee_max_count: Option<usize>) -> &mut Self {
        if let Some(basefee_max_count) = basefee_max_count {
            self.basefee_max_count = basefee_max_count;
        }
        self
    }

    pub fn basefee_max_size(&mut self, basefee_max_size: Option<usize>) -> &mut Self {
        if let Some(basefee_max_size) = basefee_max_size {
            self.basefee_max_size = basefee_max_size;
        }
        self
    }

    pub fn queued_max_count(&mut self, queued_max_count: Option<usize>) -> &mut Self {
        if let Some(queued_max_count) = queued_max_count {
            self.queued_max_count = queued_max_count;
        }
        self
    }

    pub fn queued_max_size(&mut self, queued_max_size: Option<usize>) -> &mut Self {
        if let Some(queued_max_size) = queued_max_size {
            self.queued_max_size = queued_max_size;
        }
        self
    }

    pub fn max_account_slots(&mut self, max_account_slots: Option<usize>) -> &mut Self {
        if let Some(max_account_slots) = max_account_slots {
            self.max_account_slots = max_account_slots;
        }
        self
    }

    pub fn price_bump(&mut self, price_bump: Option<u128>) -> &mut Self {
        if let Some(price_bump) = price_bump {
            self.price_bump = price_bump;
        }
        self
    }

    pub fn blob_transaction_price_bump(
        &mut self,
        blob_transaction_price_bump: Option<u128>,
    ) -> &mut Self {
        if let Some(blob_transaction_price_bump) = blob_transaction_price_bump {
            self.blob_transaction_price_bump = blob_transaction_price_bump;
        }
        self
    }

    pub fn no_locals(&mut self, no_locals: Option<bool>) -> &mut Self {
        if let Some(no_locals) = no_locals {
            self.no_locals = no_locals;
        }
        self
    }

    pub fn build(&self) -> TestCluster {
        TestCluster {
            narwhal_port: self.narwhal_port.clone(),
            metrics_addr: self.metrics_addr.clone(),
            metrics_port: self.metrics_port.clone(),
            ws_port: self.ws_port.clone(),
            ws_addr: self.ws_addr.clone(),
            http_port: self.http_port.clone(),
            http_addr: self.http_addr.clone(),
            auth_port: self.auth_port.clone(),
            auth_addr: self.auth_addr.clone(),
            auth_jwt_secret: self.auth_jwt_secret.clone(),
            instance: self.instance,
            chain: self.chain.clone(),
            data_dir: self.data_dir.clone(),
            pending_max_count: self.pending_max_count,
            pending_max_size: self.pending_max_size,
            basefee_max_count: self.basefee_max_count,
            basefee_max_size: self.basefee_max_size,
            queued_max_count: self.queued_max_count,
            queued_max_size: self.queued_max_size,
            max_account_slots: self.max_account_slots,
            price_bump: self.price_bump,
            blob_transaction_price_bump: self.blob_transaction_price_bump,
            no_locals: self.no_locals,
        }
    }
}

/// Runs the given future to completion or until a critical task panicked
async fn run_to_completion_or_panic<F, E>(
    mut tasks: TaskManager,
    fut: F,
    mut rx_shutdown: watch::Receiver<()>,
) -> Result<TaskManager, E>
where
    F: Future<Output = Result<(), E>>,
    E: Send + Sync + From<reth_tasks::PanickedTaskError> + 'static,
{
    {
        pin_mut!(fut);
        tokio::select! {
            _ = rx_shutdown.changed() => {
                info!(target: "reth::cli", "Received shutdown signal");
            },

            err = &mut tasks => {
                return Err(err.into())
            },
            res = fut => res?,
        }
    }
    Ok(tasks)
}

/// Runs the future to completion or until:
/// - `ctrl-c` is received.
/// - `SIGTERM` is received (unix only).
async fn run_until_ctrl_c<F, E>(fut: F, mut rx_shutdown: watch::Receiver<()>) -> Result<(), E>
where
    F: Future<Output = Result<(), E>>,
    E: Send + Sync + 'static + From<std::io::Error>,
{
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut stream = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let sigterm = stream.recv();
        pin_mut!(sigterm, ctrl_c, fut);

        tokio::select! {
            _ = rx_shutdown.changed() => {
                info!(target: "reth::cli", "Waiting for tasks to finish.");
            },
            _ = ctrl_c => {
                trace!(target: "reth::cli",  "Received ctrl-c");
            },
            _ = sigterm => {
                trace!(target: "reth::cli",  "Received SIGTERM");
            },
            res = fut => res?,
        }
    }

    #[cfg(not(unix))]
    {
        pin_mut!(ctrl_c, fut);

        tokio::select! {
            _ = ctrl_c => {
                trace!(target: "reth::cli",  "Received ctrl-c");
            },
            res = fut => res?,
        }
    }

    Ok(())
}
