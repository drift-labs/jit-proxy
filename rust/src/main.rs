use std::env;
use std::str::FromStr;

use anchor_client::Cluster;
use dotenv::dotenv;
use solana_sdk::signature::Keypair;

use drift_sdk::types::{CommitmentConfig, Context, RpcSendTransactionConfig};
use drift_sdk::{DriftClient, RpcAccountProvider};
use jitter::{JitParams, Jitter};
use types::ComputeBudgetParams;

pub mod jit_proxy_client;
pub mod jitter;
pub mod types;

#[tokio::main]
async fn main() {
    env_logger::init();
    dotenv().ok();

    let rpc_url = env::var("RPC_URL").expect("RPC_KEY must be set");
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY must be set");

    let pk_vec: Vec<u8> = private_key
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().parse::<u8>().expect("Failed to parse u8"))
        .collect();

    let pk_bytes: &[u8] = &pk_vec;
    let keypair = Keypair::from_bytes(pk_bytes).unwrap();

    let drift_client = DriftClient::new(
        Context::MainNet,
        RpcAccountProvider::with_commitment(&rpc_url, CommitmentConfig::finalized()),
        keypair.into(),
    )
    .await
    .unwrap();

    let config = RpcSendTransactionConfig::default();

    let cu_params = ComputeBudgetParams::new(100_000, 1_400_000);

    let jitter = Jitter::new_with_shotgun(drift_client, Some(config), Some(cu_params));

    let cluster = Cluster::from_str(&rpc_url).unwrap();
    let url = cluster.ws_url().to_string();

    let jit_params = JitParams::new(
        0,
        0,
        -1_000_000,
        1_000_000,
        jit_proxy::state::PriceType::Oracle,
    );

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
