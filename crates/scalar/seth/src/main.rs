//! This example shows how to implement a node with a custom EVM

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_primitives::{address, Address, Bytes, U256};
use clap::{Args, Parser};
use reth::{args::utils::DefaultChainSpecParser, cli::Cli};
use reth::{
    builder::{
        components::{ExecutorBuilder, PayloadServiceBuilder},
        BuilderContext,
    },
    payload::{EthBuiltPayload, EthPayloadBuilderAttributes},
    primitives::revm_primitives::{Env, PrecompileResult},
    revm::{
        handler::register::EvmHandler,
        inspector_handle_register,
        precompile::{Precompile, PrecompileOutput, PrecompileSpecId},
        primitives::BlockEnv,
        ContextPrecompiles, Database, Evm, EvmBuilder, GetInspector,
    },
    rpc::types::engine::PayloadAttributes,
    transaction_pool::TransactionPool,
};
use reth_chainspec::ChainSpec;
use reth_evm_ethereum::EthEvmConfig;
use reth_node_api::{
    ConfigureEvm, ConfigureEvmEnv, FullNodeTypes, NextBlockEnvAttributes, NodeTypes,
    NodeTypesWithEngine, PayloadTypes,
};
use reth_node_builder::{
    engine_tree_config::{
        TreeConfig, DEFAULT_MEMORY_BLOCK_BUFFER_TARGET, DEFAULT_PERSISTENCE_THRESHOLD,
    },
    EngineNodeLauncher,
};
use reth_node_ethereum::{
    node::{EthereumAddOns, EthereumPayloadBuilder},
    EthereumNode,
};
use reth_primitives::{
    revm_primitives::{CfgEnvWithHandlerCfg, TxEnv},
    Header, TransactionSigned,
};
use reth_provider::providers::BlockchainProvider2;
use reth_tracing::{RethTracer, Tracer};
use scalar_pevm::executor::{EthExecutorProvider, ParallelExecutorProvider};
use std::sync::Arc;

/// Parameters for configuring the engine
#[derive(Debug, Clone, Args, PartialEq, Eq)]
#[command(next_help_heading = "Engine")]
pub struct EngineArgs {
    /// Enable the engine2 experimental features on reth binary
    #[arg(long = "engine.experimental", default_value = "false")]
    pub experimental: bool,

    /// Configure persistence threshold for engine experimental.
    #[arg(long = "engine.persistence-threshold", requires = "experimental", default_value_t = DEFAULT_PERSISTENCE_THRESHOLD)]
    pub persistence_threshold: u64,

    /// Configure the target number of blocks to keep in memory.
    #[arg(long = "engine.memory-block-buffer-target", requires = "experimental", default_value_t = DEFAULT_MEMORY_BLOCK_BUFFER_TARGET)]
    pub memory_block_buffer_target: u64,
}

impl Default for EngineArgs {
    fn default() -> Self {
        Self {
            experimental: false,
            persistence_threshold: DEFAULT_PERSISTENCE_THRESHOLD,
            memory_block_buffer_target: DEFAULT_MEMORY_BLOCK_BUFFER_TARGET,
        }
    }
}

/// Custom EVM configuration
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ScalarEvmConfig {
    /// Wrapper around mainnet configuration
    inner: EthEvmConfig,
}

impl ScalarEvmConfig {
    pub const fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { inner: EthEvmConfig::new(chain_spec) }
    }
}

impl ScalarEvmConfig {
    /// Sets the precompiles to the EVM handler
    ///
    /// This will be invoked when the EVM is created via [ConfigureEvm::evm] or
    /// [ConfigureEvm::evm_with_inspector]
    ///
    /// This will use the default mainnet precompiles and add additional precompiles.
    pub fn set_precompiles<EXT, DB>(handler: &mut EvmHandler<EXT, DB>)
    where
        DB: Database,
    {
        // first we need the evm spec id, which determines the precompiles
        let spec_id = handler.cfg.spec_id;

        // install the precompiles
        handler.pre_execution.load_precompiles = Arc::new(move || {
            let mut precompiles = ContextPrecompiles::new(PrecompileSpecId::from_spec_id(spec_id));
            precompiles.extend([(
                address!("0000000000000000000000000000000000000999"),
                Precompile::Env(Self::my_precompile).into(),
            )]);
            precompiles
        });
    }

    /// A custom precompile that does nothing
    fn my_precompile(_data: &Bytes, _gas: u64, _env: &Env) -> PrecompileResult {
        Ok(PrecompileOutput::new(0, Bytes::new()))
    }
}

impl ConfigureEvmEnv for ScalarEvmConfig {
    type Header = Header;

    fn fill_tx_env(&self, tx_env: &mut TxEnv, transaction: &TransactionSigned, sender: Address) {
        self.inner.fill_tx_env(tx_env, transaction, sender);
    }

    fn fill_tx_env_system_contract_call(
        &self,
        env: &mut Env,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) {
        self.inner.fill_tx_env_system_contract_call(env, caller, contract, data);
    }

    fn fill_cfg_env(
        &self,
        cfg_env: &mut CfgEnvWithHandlerCfg,
        header: &Self::Header,
        total_difficulty: U256,
    ) {
        self.inner.fill_cfg_env(cfg_env, header, total_difficulty);
    }

    fn next_cfg_and_block_env(
        &self,
        parent: &Self::Header,
        attributes: NextBlockEnvAttributes,
    ) -> (CfgEnvWithHandlerCfg, BlockEnv) {
        self.inner.next_cfg_and_block_env(parent, attributes)
    }
}

impl ConfigureEvm for ScalarEvmConfig {
    type DefaultExternalContext<'a> = ();

    fn evm<DB: Database>(&self, db: DB) -> Evm<'_, Self::DefaultExternalContext<'_>, DB> {
        EvmBuilder::default()
            .with_db(db)
            // add additional precompiles
            .append_handler_register(ScalarEvmConfig::set_precompiles)
            .build()
    }

    fn evm_with_inspector<DB, I>(&self, db: DB, inspector: I) -> Evm<'_, I, DB>
    where
        DB: Database,
        I: GetInspector<DB>,
    {
        EvmBuilder::default()
            .with_db(db)
            .with_external_context(inspector)
            // add additional precompiles
            .append_handler_register(ScalarEvmConfig::set_precompiles)
            .append_handler_register(inspector_handle_register)
            .build()
    }

    fn default_external_context<'a>(&self) -> Self::DefaultExternalContext<'a> {}
}

/// Builds a regular ethereum block executor that uses the custom EVM.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct SequentialExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for SequentialExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec>>,
{
    type EVM = ScalarEvmConfig;
    type Executor = EthExecutorProvider<Self::EVM>;
    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        Ok((
            ScalarEvmConfig::new(ctx.chain_spec()),
            EthExecutorProvider::new(ctx.chain_spec(), ScalarEvmConfig::new(ctx.chain_spec())),
        ))
    }
}

/// Builds a regular ethereum block executor that uses the custom EVM.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec>>,
{
    type EVM = ScalarEvmConfig;
    type Executor = ParallelExecutorProvider<Self::EVM>;
    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let chain_spec = ctx.chain_spec();
        Ok((
            ScalarEvmConfig::new(chain_spec.clone()),
            ParallelExecutorProvider::new(chain_spec.clone(), ScalarEvmConfig::new(chain_spec)),
        ))
    }
}

/// Builds a regular ethereum block executor that uses the custom EVM.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ScalarPayloadBuilder {
    inner: EthereumPayloadBuilder,
}

impl<Types, Node, Pool> PayloadServiceBuilder<Node, Pool> for ScalarPayloadBuilder
where
    Types: NodeTypesWithEngine<ChainSpec = ChainSpec>,
    Node: FullNodeTypes<Types = Types>,
    Pool: TransactionPool + Unpin + 'static,
    Types::Engine: PayloadTypes<
        BuiltPayload = EthBuiltPayload,
        PayloadAttributes = PayloadAttributes,
        PayloadBuilderAttributes = EthPayloadBuilderAttributes,
    >,
{
    async fn spawn_payload_service(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<reth::payload::PayloadBuilderHandle<Types::Engine>> {
        self.inner.spawn(ScalarEvmConfig::new(ctx.chain_spec()), ctx, pool)
    }
}
#[tokio::main]
async fn main() -> eyre::Result<()> {
    let _guard = RethTracer::new().init()?;
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }
    let res =
        Cli::<DefaultChainSpecParser, EngineArgs>::parse().run(|builder, engine_args| async move {
            let enable_engine2 = engine_args.experimental;
            match enable_engine2 {
                true => {
                    let engine_tree_config = TreeConfig::default()
                        .with_persistence_threshold(engine_args.persistence_threshold)
                        .with_memory_block_buffer_target(engine_args.memory_block_buffer_target);
                    let handle = builder
                        .with_types_and_provider::<EthereumNode, BlockchainProvider2<_>>()
                        .with_components(
                            EthereumNode::components()
                                .executor(ParallelExecutorBuilder::default())
                                .payload(ScalarPayloadBuilder::default()),
                        )
                        .with_add_ons::<EthereumAddOns>()
                        .launch_with_fn(|builder| {
                            let launcher = EngineNodeLauncher::new(
                                builder.task_executor().clone(),
                                builder.config().datadir(),
                                engine_tree_config,
                            );
                            builder.launch_with(launcher)
                        })
                        .await?;
                    handle.node_exit_future.await
                }
                false => {
                    let handle = builder.launch_node(EthereumNode::default()).await?;
                    handle.node_exit_future.await
                }
            }
        });
    if let Err(err) = res {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
    res
}
