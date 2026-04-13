//! Substrate RPC client with dynamic metadata decoding.

use async_trait::async_trait;
use futures::StreamExt;
use subxt::rpcs::client::reconnecting_rpc_client::RpcClient as ReconnectingRpcClient;
use subxt::rpcs::RpcClient;
use subxt::client::{Block, OnlineClientAtBlock};
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
    ///
    /// Follows the canonical flow from `subxt/examples/rpc_client.rs`:
    /// `ReconnectingRpcClient::builder().build(url)` → `RpcClient::new` →
    /// `OnlineClient::from_rpc_client`. Both `ws://` and `wss://` URLs are
    /// accepted.
    #[instrument(skip_all, fields(url = %config.ws_url))]
    pub async fn connect(config: SubstrateClientConfig) -> ChainResult<Self> {
        debug!("Connecting to node");

        let inner = ReconnectingRpcClient::builder()
            .build(&config.ws_url)
            .await
            .map_err(|e| ChainError::ConnectionFailed(e.to_string()))?;
        let rpc_client = RpcClient::new(inner);
        let client = OnlineClient::<PolkadotConfig>::from_rpc_client(rpc_client)
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

    /// Current finalized head.
    async fn finalized_head(&self) -> ChainResult<FinalizedHead> {
        self.current_head().await
    }

    /// Current best head.
    ///
    /// The 0.50 `ChainHead` backend carries the finalized/best distinction on
    /// the subscription stream, not on the point-in-time accessor. This method
    /// therefore returns the finalized head (lags the best head by the
    /// finalization gap). The only caller is a startup debug log; the streaming
    /// path uses `subscribe_best` and is unaffected.
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
