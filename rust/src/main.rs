use std::{str::FromStr, sync::Arc};

use solana_sdk::signature::Keypair;
use thiserror::Error;
use drift_sdk::{types::{CommitmentConfig, Context, SdkError}, DriftClient, RpcAccountProvider};
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

    let api_key = env::var("RPC_KEY").expect("RPC_KEY must be set");
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY must be set");

    let pk_vec: Vec<u8> = private_key.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().parse::<u8>().expect("Failed to parse u8"))
        .collect();

    let pk_bytes: &[u8] = &pk_vec;
    let rpc_url = format!("https://mainnet.helius-rpc.com?api-key={}", api_key);
    let keypair = Keypair::from_bytes(pk_bytes).unwrap();

    let drift_client = DriftClient::new(
        Context::MainNet,
        RpcAccountProvider::with_commitment(&rpc_url, CommitmentConfig::finalized()),
        keypair.into(),
    )
    .await
    .unwrap();

    let jit_proxy_client = JitProxyClient::new(drift_client.clone()).await;

    let jit_params = JitParams::new(
        0,
        0,
        -1_000_000,
        1_000_000,
        jit_proxy::state::PriceType::Oracle,
    );

    let shotgun: Arc<dyn JitterStrategy + Send + Sync> = Arc::new(Shotgun { jit_proxy_client });
    
    let jitter = jitter::Jitter::new(drift_client.clone(), shotgun);

    let cluster = Cluster::from_str(&rpc_url).unwrap();
    let url = cluster.ws_url().to_string();

    jitter.update_perp_params(0, jit_params.clone());
    jitter.update_perp_params(1, jit_params.clone());
    jitter.update_perp_params(2, jit_params.clone());
    jitter.update_perp_params(3, jit_params.clone());
    jitter.update_perp_params(4, jit_params.clone());
    jitter.update_perp_params(5, jit_params.clone());
    jitter.update_perp_params(6, jit_params.clone());
    jitter.update_perp_params(7, jit_params.clone());
    jitter.update_perp_params(8, jit_params.clone());
    jitter.update_perp_params(9, jit_params.clone());
    jitter.update_perp_params(10, jit_params.clone());

    let _ = jitter.subscribe(url).await.unwrap();
}
