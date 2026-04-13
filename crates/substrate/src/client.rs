//! Substrate RPC client with dynamic metadata decoding.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use subxt::backend::{ChainHeadBackend, ChainHeadBackendBuilder};
use subxt::client::{Block, OnlineClientAtBlock};
use subxt::rpcs::RpcClient;
use subxt::{OnlineClient, PolkadotConfig};
use tracing::{debug, instrument};

use maestro_core::error::{ChainError, ChainResult};
use maestro_core::models::BlockHash;
use maestro_core::ports::{BlockSource, FinalizedBlockStream, FinalizedHead};

use crate::decode::decode_raw_block;

/// Configuration for the Substrate client.
#[derive(Debug, Clone)]
pub struct SubstrateClientConfig {
    /// WebSocket URL (e.g., "ws://localhost:9944").
    pub ws_url: String,
}

pub type SubstrateBlock = Block<PolkadotConfig>;
pub(crate) type SubstrateClientAtBlock = OnlineClientAtBlock<PolkadotConfig>;

impl Default for SubstrateClientConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://127.0.0.1:9944".to_string(),
        }
    }
}

/// Substrate client adapter implementing the BlockSource port.
pub struct SubstrateClient {
    pub(crate) client: OnlineClient<PolkadotConfig>,
}

impl SubstrateClient {
    /// Connect to a Substrate node.
    #[instrument(skip_all, fields(url = %config.ws_url))]
    pub async fn connect(config: SubstrateClientConfig) -> ChainResult<Self> {
        debug!("Connecting to node");

        // Use insecure client for ws:// URLs, secure client for wss://
        let rpc_client = if config.ws_url.starts_with("ws://") {
            RpcClient::from_insecure_url(&config.ws_url)
                .await
                .map_err(|e| ChainError::ConnectionFailed(e.to_string()))?
        } else {
            RpcClient::from_url(&config.ws_url)
                .await
                .map_err(|e| ChainError::ConnectionFailed(e.to_string()))?
        };

        // Phase 2 hook: flip `use_historic_types` to `true` when historical sync lands.
        let chain_config = PolkadotConfig::builder().use_historic_types(false).build();

        let backend: ChainHeadBackend<PolkadotConfig> =
            ChainHeadBackendBuilder::default().build_with_background_driver(rpc_client.clone());

        let client = OnlineClient::<PolkadotConfig>::from_backend_with_config(
            chain_config,
            Arc::new(backend),
        )
        .await
        .map_err(|e| ChainError::ConnectionFailed(e.to_string()))?;

        debug!("Connected successfully");

        Ok(Self { client })
    }

    async fn current_head(&self) -> ChainResult<FinalizedHead> {
        let at_block = self
            .client
            .at_current_block()
            .await
            .map_err(|e| ChainError::RpcError(e.to_string()))?;

        Ok(FinalizedHead {
            number: at_block.block_number(),
            hash: at_block.block_hash().into(),
        })
    }
}

#[async_trait]
impl BlockSource for SubstrateClient {
    async fn genesis_hash(&self) -> ChainResult<BlockHash> {
        let hash = self.client.genesis_hash();
        Ok(BlockHash(hash.0))
    }

    /// Current finalized block.
    ///
    /// In subxt 0.50, `OnlineClient::at_current_block()` returns the block at
    /// the current finalized head under the `ChainHead` backend. This is a
    /// silent bugfix versus the pre-0.50 code, which called `blocks().at_latest()`
    /// — under `ChainHead` that returned the best block, not the finalized one,
    /// so `finalized_head` previously over-reported by the finalization gap.
    async fn finalized_head(&self) -> ChainResult<FinalizedHead> {
        self.current_head().await
    }

    /// Current best block.
    ///
    /// The subxt 0.50 `ChainHead` backend does not expose a direct "best head"
    /// accessor distinct from `at_current_block` — the finalized/best distinction
    /// is carried entirely by the subscription stream (`stream_blocks` vs
    /// `stream_best_blocks`). Consequently this method returns the finalized
    /// head, which lags the best head by the finalization gap. In practice the
    /// only caller is the startup debug log in `services/indexer.rs`; the
    /// streaming path uses `subscribe_best` and is unaffected.
    async fn best_head(&self) -> ChainResult<FinalizedHead> {
        self.current_head().await
    }

    async fn subscribe_finalized(&self) -> ChainResult<FinalizedBlockStream> {
        let stream = self
            .client
            .stream_blocks()
            .await
            .map_err(|e| ChainError::SubscriptionError(e.to_string()))?;

        let mapped = stream.then(|result| async move {
            match result {
                Ok(block) => decode_raw_block(&block).await,
                Err(e) => Err(ChainError::SubscriptionError(e.to_string())),
            }
        });

        Ok(Box::pin(mapped))
    }

    async fn subscribe_best(&self) -> ChainResult<FinalizedBlockStream> {
        let stream = self
            .client
            .stream_best_blocks()
            .await
            .map_err(|e| ChainError::SubscriptionError(e.to_string()))?;

        let mapped = stream.then(|result| async move {
            match result {
                Ok(block) => decode_raw_block(&block).await,
                Err(e) => Err(ChainError::SubscriptionError(e.to_string())),
            }
        });

        Ok(Box::pin(mapped))
    }

    async fn runtime_version(&self) -> ChainResult<u32> {
        let at_block = self
            .client
            .at_current_block()
            .await
            .map_err(|e| ChainError::RpcError(e.to_string()))?;
        Ok(at_block.spec_version())
    }
}
