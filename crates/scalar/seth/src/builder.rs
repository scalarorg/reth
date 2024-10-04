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
use scalar_pevm::executor::parallel::ParallelEvmContext;
use scalar_pevm::executor::{EthExecutorProvider, ParallelExecutorProvider};
use std::sync::Arc;

use crate::ScalarEvmConfig;

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
        Ok((ParallelExecutorProvider::new(chain_spec.clone(), ScalarEvmConfig::new(chain_spec)),))
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
