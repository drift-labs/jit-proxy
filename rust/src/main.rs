use std::{str::FromStr, sync::Arc};

use solana_sdk::signature::Keypair;
use thiserror::Error;
use drift_sdk::{types::{Context, SdkError}, DriftClient, RpcAccountProvider};
use anchor_client::Cluster;

pub mod jit_proxy_client;
pub use jit_proxy_client::JitProxyClient;

use crate::jitter::{JitterStrategy, Shotgun, JitParams};
pub mod jitter;

pub type JitResult<T> = Result<T, JitError>;

#[derive(Debug, Error)]
pub enum JitError {
    #[error("{0}")]
    Drift(String),
    #[error("{0}")]
    Sdk(String),
}


impl From<drift::error::ErrorCode> for JitError {
    fn from(error: drift::error::ErrorCode) -> Self {
        JitError::Drift(error.to_string())
    }
}

impl From<SdkError> for JitError {
    fn from(error: SdkError) -> Self {
        JitError::Sdk(error.to_string())
    }
}

const MAINNET_ENDPOINT: &str = "https://mainnet.helius-rpc.com?api-key=";

#[tokio::main]
async fn main() {
    env_logger::init();

    let drift_client = DriftClient::new(
        Context::MainNet,
        RpcAccountProvider::new(MAINNET_ENDPOINT),
        Keypair::new().into(),
    )
    .await
    .unwrap();

    let jit_proxy_client = JitProxyClient::new(drift_client).await;

    let jit_params = JitParams::new(
        0,
        0,
        -1_000_000,
        1_000_000,
        jit_proxy::state::PriceType::Oracle,
        None,
    );

    let shotgun: Arc<dyn JitterStrategy + Send + Sync> = Arc::new(Shotgun);
    
    let jitter = jitter::Jitter::new(jit_proxy_client, shotgun);

    let cluster = Cluster::from_str(MAINNET_ENDPOINT).unwrap();
    let url = cluster.ws_url().to_string();

    jitter.update_perp_params(0, jit_params);

    let _ = jitter.subscribe(url).await.unwrap();
}
