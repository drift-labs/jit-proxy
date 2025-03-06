//! Simple example JIT(er)
//!
//! It watches jit auctions and fastlane orders, and tries to fill them using the jit-proxy program.
//! jit-proxy program will ensure fills at set prices and max/min positions (otherwise fail)
//! prices are set at a fixed margin from oracle, fills are attempted every slot until an auction is complete
//!
//! This is provided as an example of the overall flow of JITing, a successful jit maker will require tuning
//! for optimal prices and tx inclusion.
//!
use std::env;

use drift_rs::{
    event_subscriber::RpcClient,
    jit_client::{ComputeBudgetParams, JitIxParams, PriceType},
    types::{CommitmentConfig, Context, RpcSendTransactionConfig},
    utils::get_ws_url,
    DriftClient, Wallet,
};

pub mod jitter;
pub mod types;

use crate::jitter::Jitter;

#[tokio::main]
async fn main() {
    env_logger::init();
    let _ = dotenv::dotenv();

    let rpc_url = env::var("RPC_URL").expect("RPC_URL must be set");
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY must be set");
    let wallet = Wallet::try_from_str(&private_key).expect("loaded wallet");

    let drift_client = DriftClient::new(
        Context::MainNet,
        RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed()),
        wallet,
    )
    .await
    .unwrap();

    let config = RpcSendTransactionConfig::default();
    let cu_params = ComputeBudgetParams::new(100_000, 1_400_000);
    let jitter = Jitter::new_with_shotgun(drift_client.clone(), Some(config), Some(cu_params));
    // some fixed width from oracle
    // IRL adjust per market and requirements
    let jit_params = JitIxParams::new(0, 0, -1_000_000, 1_000_000, PriceType::Oracle, None);

    // try to JIT make on the first 10 perp markets
    for market_idx in 0..=10 {
        jitter.update_perp_params(market_idx, jit_params.clone());
    }
    let fwog_perp = drift_client.market_lookup("fwog-perp").unwrap();
    jitter.update_perp_params(fwog_perp.index(), jit_params.clone());

    let _auction_subscriber = jitter
        .subscribe(get_ws_url(&rpc_url).expect("valid RPC url"))
        .await
        .unwrap();

    let _ = tokio::signal::ctrl_c().await;
    log::info!("jitter shutting down...");
}
