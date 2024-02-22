use std::{str::FromStr, sync::Arc};

use solana_sdk::signature::Keypair;
use thiserror::Error;
use drift_sdk::{types::{Context, SdkError}, DriftClient, RpcAccountProvider};
use anchor_client::Cluster;

pub mod jit_proxy_client;
pub use jit_proxy_client::JitProxyClient;

use crate::jitter::{JitterStrategy, Shotgun, JitParams};
pub mod jitter;
use dotenv::dotenv;
use std::env;

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

#[tokio::main]
async fn main() {
    env_logger::init();
    dotenv().ok();

    let api_key = env::var("RPC").expect("RPC must be set");
    let rpc_url = format!("https://mainnet.helius-rpc.com?api-key={}", api_key);

    let drift_client = DriftClient::new(
        Context::MainNet,
        RpcAccountProvider::new(&rpc_url),
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

    let cluster = Cluster::from_str(&rpc_url).unwrap();
    let url = cluster.ws_url().to_string();

    jitter.update_perp_params(0, jit_params);

    let _ = jitter.subscribe(url).await.unwrap();
}
