use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use dashmap::DashMap;
use drift::state::user::{OrderStatus, User, UserStats};
use drift_sdk::{
    auction_subscriber::{AuctionSubscriber, AuctionSubscriberConfig},
    constants::PROGRAM_ID as drift_program,
    event_emitter::Event,
    slot_subscriber::SlotSubscriber,
    types::{
        CommitmentConfig, MarketType, Order, PerpMarket, ReferrerInfo, RpcSendTransactionConfig,
    },
    websocket_program_account_subscriber::ProgramAccountUpdate,
    AccountProvider, DriftClient, Pubkey, Wallet,
};
pub use jit_proxy::state::PriceType;
use solana_sdk::signature::Signature;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;

use crate::types::JitResult;
use crate::{
    jit_proxy_client::{JitIxParams, JitProxyClient},
    types::ComputeBudgetParams,
};

pub type ExcludeAuctionFn = dyn Fn(&User, &String, Order) -> bool + Send + Sync;

#[inline(always)]
fn log_details(order: &Order) {
    log::info!(
        "Order Details:\n\
        Market Type: {:?}\n\
        Market index: {}\n\
        Order price: {}\n\
        Order direction: {:?}\n\
        Auction start price: {}\n\
        Auction end price: {}\n\
        Auction duration: {} slots\n\
        Order base asset amount: {}\n\
        Order base asset amount filled: {}",
        order.market_type,
        order.market_index,
        order.price,
        order.direction,
        order.auction_start_price,
        order.auction_end_price,
        order.auction_duration,
        order.base_asset_amount,
        order.base_asset_amount_filled
    );
}

#[inline(always)]
fn check_err(err: String, order_sig: String) -> Option<()> {
    if err.contains("0x1770") || err.contains("0x1771") {
        log::error!("Order: {} does not cross params yet, retrying", order_sig);
        None
    } else if err.contains("0x1779") {
        log::error!("Order: {} could not fill, retrying", order_sig);
        None
    } else if err.contains("0x1793") {
        log::error!("Oracle invalid, retrying: {}", order_sig);
        None
    } else if err.contains("0x1772") {
        log::error!("Order: {} already filled", order_sig);
        Some(())
    } else {
        log::error!("Order: {}, Error: {}", order_sig, err);
        Some(())
    }
}

#[derive(Clone)]
pub struct JitParams {
    bid: i64,
    ask: i64,
    min_position: i64,
    max_position: i64,
    price_type: PriceType,
}

impl JitParams {
    pub fn new(
        bid: i64,
        ask: i64,
        min_position: i64,
        max_position: i64,
        price_type: PriceType,
    ) -> Self {
        Self {
            bid,
            ask,
            min_position,
            max_position,
            price_type,
        }
    }
}

pub struct Jitter<T: AccountProvider> {
    drift_client: DriftClient<T>,
    perp_params: DashMap<u16, JitParams>,
    spot_params: DashMap<u16, JitParams>,
    ongoing_auctions: DashMap<String, JoinHandle<()>>,
    exclusion_criteria: AtomicBool,
    jitter: Arc<dyn JitterStrategy + Send + Sync>,
}

#[async_trait]
pub trait JitterStrategy {
    async fn try_fill(
        &self,
        taker: User,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: String,
        referrer_info: Option<ReferrerInfo>,
        params: JitParams,
        now: std::time::Instant,
    ) -> JitResult<()>;
}

impl<T: AccountProvider + Clone> Jitter<T> {
    pub fn new(
        drift_client: DriftClient<T>,
        jitter: Arc<dyn JitterStrategy + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Jitter {
            drift_client,
            perp_params: DashMap::new(),
            spot_params: DashMap::new(),
            ongoing_auctions: DashMap::new(),
            exclusion_criteria: AtomicBool::new(false),
            jitter,
        })
    }

    /// Set up a Jitter with the Shotgun strategy
    pub fn new_with_shotgun(
        drift_client: DriftClient<T>,
        config: Option<RpcSendTransactionConfig>,
        cu_params: Option<ComputeBudgetParams>,
    ) -> Arc<Self> {
        let jit_proxy_client = JitProxyClient::new(drift_client.clone(), config, cu_params);
        let shotgun: Arc<dyn JitterStrategy + Send + Sync> = Arc::new(Shotgun { jit_proxy_client });

        Arc::new(Jitter {
            drift_client,
            perp_params: DashMap::new(),
            spot_params: DashMap::new(),
            ongoing_auctions: DashMap::new(),
            exclusion_criteria: AtomicBool::new(false),
            jitter: shotgun,
        })
    }

    // Subscribe to auction events and start listening for them
    pub async fn subscribe(self: Arc<Self>, url: String) -> JitResult<()> {
        let (auction_sender, mut auction_receiver): (
            Sender<Box<(dyn Event)>>,
            Receiver<Box<dyn Event>>,
        ) = mpsc::channel(100);

        // Set up a receiver to bounce the auction events to the trampoline
        tokio::spawn(async move {
            while let Some(event) = auction_receiver.recv().await {
                let _ = self.on_auction_sync(event);
            }
        });

        let auction_subscriber_config = AuctionSubscriberConfig {
            commitment: CommitmentConfig::processed(),
            resub_timeout_ms: None,
            url,
        };

        let mut auction_subscriber = AuctionSubscriber::new(auction_subscriber_config);
        auction_subscriber.subscribe().await?;

        let auction_sender: Sender<Box<dyn Event>> = auction_sender.clone();

        // Send auction events to the receiver to be processed
        auction_subscriber
            .event_emitter
            .subscribe("auction", move |event| {
                let _ = auction_sender.try_send(event);
            });

        futures::future::pending::<()>().await; // Keep the event loop alive forever

        Ok(())
    }

    // Simple trampoline to the async function we need from the auction event
    pub fn on_auction_sync(self: &Arc<Self>, event: Box<dyn Event>) -> JitResult<()> {
        let jitter = self.clone();
        tokio::spawn(async move {
            let _ = jitter.on_auction(event).await;
        });

        Ok(())
    }

    // Process the auction event & attempt to fill with JIT if possible
    pub async fn on_auction(&self, event: Box<dyn Event>) -> JitResult<()> {
        if let Some(auction) = event.as_any().downcast_ref::<ProgramAccountUpdate<User>>() {
            log::info!("Auction received");
            let now = std::time::Instant::now();

            let user_pubkey = &auction.pubkey;
            let user = auction.data_and_slot.data.clone();
            let user_stats_key = Wallet::derive_stats_account(&user.authority, &drift_program);

            for order in user.orders {
                if order.status != OrderStatus::Open || !order.has_auction() || self.exclusion_criteria.load(Ordering::Relaxed) {
                    continue;
                }

                let order_sig = self.get_order_signatures(&user_pubkey, order.order_id);

                if self.ongoing_auctions.contains_key(&order_sig) {
                    continue;
                }

                match order.market_type {
                    MarketType::Perp => {
                        if let Some(param) = self.perp_params.get(&order.market_index) {
                            let perp_market: PerpMarket = self
                                .drift_client
                                .get_perp_market_account(order.market_index).expect("perp market");
                            let remaining = order.base_asset_amount - order.base_asset_amount_filled;
                            let min_order_size = perp_market.amm.min_order_size;

                            if remaining < min_order_size {
                                log::warn!(
                                    "Order filled within min order size\nRemaining: {}\nMinimum order size: {}",
                                    remaining,
                                    min_order_size
                                );
                                return Ok(());
                            }

                            if (remaining as i128) < param.min_position.into() {
                                log::warn!(
                                    "Order filled within min position\nRemaining: {}\nMin position: {}",
                                    remaining,
                                    param.min_position
                                );
                                return Ok(());
                            }

                            let jitter = self.jitter.clone();
                            let taker = user_pubkey.clone();
                            let order_signature = order_sig.clone();

                            let taker_stats: UserStats = self
                                .drift_client
                                .get_user_stats(&user.authority)
                                .await
                                .expect("user stats");
                            let referrer_info = ReferrerInfo::get_referrer_info(taker_stats);

                            let param = param.clone();
                            let ongoing_auction = tokio::spawn(async move {
                                let _ = jitter
                                    .try_fill(
                                        user.clone(),
                                        Pubkey::from_str(&taker).unwrap(),
                                        user_stats_key.clone(),
                                        order.clone(),
                                        order_signature.clone(),
                                        referrer_info,
                                        param.clone(),
                                        now
                                    )
                                    .await;
                            });

                            self.ongoing_auctions.insert(order_sig, ongoing_auction);
                        } else {
                            log::warn!("Jitter not listening to {}", order.market_index);
                            return Ok(());
                        }
                    }
                    MarketType::Spot => {
                        if let Some(param) = self.spot_params.get(&order.market_index) {
                            let spot_market = self
                                .drift_client
                                .get_spot_market_account(order.market_index)
                                .expect("spot market");

                            if order.base_asset_amount - order.base_asset_amount_filled
                                < spot_market.min_order_size
                            {
                                log::warn!(
                                    "Order filled within min order size\nRemaining: {}\nMinimum order size: {}",
                                    order.base_asset_amount - order.base_asset_amount_filled,
                                    spot_market.min_order_size
                                );
                                return Ok(());
                            }

                            if (order.base_asset_amount as i128)
                                - (order.base_asset_amount_filled as i128)
                                < param.min_position.into()
                            {
                                log::warn!(
                                    "Order filled within min order size\nRemaining: {}\nMinimum order size: {}",
                                    order.base_asset_amount - order.base_asset_amount_filled,
                                    spot_market.min_order_size
                                );
                                return Ok(());
                            }

                            let jitter = self.jitter.clone();
                            let taker = user_pubkey.clone();
                            let order_signature = order_sig.clone();

                            let taker_stats: UserStats =
                                self.drift_client.get_user_stats(&user.authority).await?;
                            let referrer_info = ReferrerInfo::get_referrer_info(taker_stats);

                            let param = param.clone();
                            let ongoing_auction = tokio::spawn(async move {
                                let _ = jitter
                                    .try_fill(
                                        user.clone(),
                                        Pubkey::from_str(&taker).unwrap(),
                                        user_stats_key.clone(),
                                        order.clone(),
                                        order_signature.clone(),
                                        referrer_info,
                                        param.clone(),
                                        now
                                    )
                                    .await;
                            });

                            self.ongoing_auctions.insert(order_sig, ongoing_auction);
                        } else {
                            log::warn!("Jitter not listening to {}", order.market_index);
                            return Ok(());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // Helper functions
    pub fn update_perp_params(&self, market_index: u16, params: JitParams) {
        self.perp_params.insert(market_index, params);
    }

    pub fn update_spot_params(&self, market_index: u16, params: JitParams) {
        self.spot_params.insert(market_index, params);
    }

    pub fn set_exclusion_criteria(&self, exclusion_criteria: bool) {
        self.exclusion_criteria
            .store(exclusion_criteria, Ordering::Relaxed);
    }

    pub fn get_order_signatures(&self, taker_key: &str, order_id: u32) -> String {
        format!("{}-{}", taker_key, order_id)
    }
}

/// Aggressive JIT making strategy
pub struct Shotgun<T: AccountProvider> {
    pub jit_proxy_client: JitProxyClient<T>,
}

/// Implementing the Sniper is left as an exercise for the reader.
/// Good luck!
pub struct Sniper<T: AccountProvider> {
    pub jit_proxy_client: JitProxyClient<T>,
    pub slot_subscriber: SlotSubscriber,
}

#[async_trait]
impl<T: AccountProvider> JitterStrategy for Shotgun<T> {
    async fn try_fill(
        &self,
        taker: User,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: String,
        referrer_info: Option<ReferrerInfo>,
        params: JitParams,
        now: std::time::Instant,
    ) -> JitResult<()> {
        log::info!("Trying to fill with Shotgun:");
        log_details(&order);

        for i in 0..order.auction_duration {
            let referrer_info = referrer_info.clone();
            log::info!(
                "Trying to fill: {:?} -> Attempt: {}",
                order_sig.clone(),
                i + 1
            );

            if params.max_position == 0 || params.min_position == 0 {
                log::warn!(
                    "min or max position is 0 -> min: {} max: {}",
                    params.min_position,
                    params.max_position
                );
                return Ok(());
            }

            let jit_ix_params = JitIxParams::new(
                taker_key.clone(),
                taker_stats_key.clone(),
                taker.clone(),
                order.order_id,
                params.max_position,
                params.min_position,
                params.bid,
                params.ask,
                Some(params.price_type),
                referrer_info,
                None,
            );

            let jit_result = self.jit_proxy_client.jit(jit_ix_params, now).await;

            match jit_result {
                Ok(sig) => {
                    if sig == Signature::default() {
                        continue;
                    }
                    log::info!("Order filled");
                    log::info!("Signature: {:?}", sig);
                    return Ok(());
                }
                Err(e) => {
                    let err = e.to_string();
                    match check_err(err, order_sig.clone()) {
                        Some(_) => return Ok(()),
                        None => {
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
