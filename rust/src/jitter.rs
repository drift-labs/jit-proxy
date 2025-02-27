use std::{
    fmt::Display,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use dashmap::DashMap;
use drift_rs::{
    auction_subscriber::{AuctionSubscriber, AuctionSubscriberConfig},
    fastlane_order_subscriber::SignedOrderInfo,
    jit_client::{ComputeBudgetParams, JitIxParams, JitProxyClient, JitTakerParams},
    slot_subscriber::SlotSubscriber,
    types::{
        accounts::{User, UserStats},
        CommitmentConfig, MarketType, Order, OrderStatus, ReferrerInfo, RpcSendTransactionConfig,
    },
    websocket_program_account_subscriber::ProgramAccountUpdate,
    DriftClient, Pubkey, Wallet,
};
use futures::StreamExt;
use solana_sdk::signature::Signature;
use tokio::task::JoinHandle;

use crate::types::{JitError, JitResult};

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
fn check_err(err: String, order_sig: &str) -> Option<()> {
    if err.contains("0x1770") || err.contains("0x1771") {
        log::error!("Order: {order_sig} does not cross params yet, retrying");
        None
    } else if err.contains("0x1779") {
        log::error!("Order: {order_sig} could not fill, retrying");
        None
    } else if err.contains("0x1793") {
        log::error!("Oracle invalid, retrying: {order_sig}");
        None
    } else if err.contains("0x1772") {
        log::error!("Order: {order_sig} already filled");
        Some(())
    } else {
        log::error!("Order: {order_sig}, Error: {err}");
        Some(())
    }
}

pub struct Jitter {
    drift_client: DriftClient,
    perp_params: DashMap<u16, JitIxParams>,
    spot_params: DashMap<u16, JitIxParams>,
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
        params: JitIxParams,
    ) -> JitResult<()>;
    async fn try_fastlane_fill(
        &self,
        signed_order_info: &SignedOrderInfo,
        order_sig: String,
        taker_params: &JitTakerParams,
        jit_params: &JitIxParams,
    ) -> JitResult<()>;
}

impl Jitter {
    pub fn new(
        drift_client: DriftClient,
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
        drift_client: DriftClient,
        config: Option<RpcSendTransactionConfig>,
        cu_params: Option<ComputeBudgetParams>,
    ) -> Arc<Self> {
        let jit_proxy_client = JitProxyClient::new(drift_client.clone(), config, cu_params);
        let shotgun = Arc::new(Shotgun {
            jit_proxy_client,
            authority: *drift_client.wallet().authority(),
        });

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
    pub async fn subscribe(self: Arc<Self>, url: String) -> JitResult<AuctionSubscriber> {
        // start fastlane order subscriber
        let markets = self.drift_client.get_all_perp_market_ids();
        let mut fastlane_order_stream = self
            .drift_client
            .subscribe_fastlane_orders(&markets)
            .await
            .map_err(|err| {
                log::warn!("failed to start fastlane subscriber: {err:?}");
                JitError::Sdk(err.to_string())
            })?;

        let self_ref = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some(signed_order_info) = fastlane_order_stream.next().await {
                if let Err(err) = self_ref.on_fastlane_order(signed_order_info).await {
                    log::warn!("processing fastlane order failed: {err:?}");
                }
            }
        });

        // start jit order subscriber
        let auction_subscriber_config = AuctionSubscriberConfig {
            commitment: CommitmentConfig::processed(),
            resub_timeout_ms: None,
            url,
        };
        let auction_subscriber = AuctionSubscriber::new(auction_subscriber_config);
        auction_subscriber.subscribe(move |auction| {
            let self_ref = Arc::clone(&self);
            let auction = auction.clone();
            tokio::spawn({
                async move {
                    if let Err(err) = self_ref.on_auction(auction).await {
                        log::warn!("processing auction failed: {err:?}");
                    }
                }
            });
        });

        Ok(auction_subscriber)
    }

    // Process the auction event & attempt to fill with JIT if possible
    pub async fn on_auction(&self, auction: ProgramAccountUpdate<User>) -> JitResult<()> {
        log::info!("Auction received");
        let user = auction.data_and_slot.data;
        let user_pubkey = &auction.pubkey;
        let user_stats_key = Wallet::derive_stats_account(&user.authority);

        for order in user.orders {
            if order.status != OrderStatus::Open
                || order.auction_duration == 0
                || self.exclusion_criteria.load(Ordering::Relaxed)
            {
                continue;
            }

            let order_sig = self.get_order_signatures(user_pubkey, order.order_id);

            if self.ongoing_auctions.contains_key(&order_sig) {
                continue;
            }

            match order.market_type {
                MarketType::Perp => {
                    if let Some(param) = self.perp_params.get(&order.market_index) {
                        let perp_market = self
                            .drift_client
                            .program_data()
                            .perp_market_config_by_index(order.market_index)
                            .unwrap();
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

                        let jitter = Arc::clone(&self.jitter);
                        let taker = Pubkey::from_str(&user_pubkey).unwrap();
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
                                    user,
                                    taker,
                                    user_stats_key,
                                    order,
                                    order_signature,
                                    referrer_info,
                                    param,
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
                            .program_data()
                            .spot_market_config_by_index(order.market_index)
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

                        let jitter = Arc::clone(&self.jitter);
                        let taker = Pubkey::from_str(&user_pubkey).unwrap();
                        let order_signature = order_sig.clone();

                        let taker_stats: UserStats =
                            self.drift_client.get_user_stats(&user.authority).await?;
                        let referrer_info = ReferrerInfo::get_referrer_info(taker_stats);

                        let param = param.clone();
                        let ongoing_auction = tokio::spawn(async move {
                            let _ = jitter
                                .try_fill(
                                    user,
                                    taker,
                                    user_stats_key,
                                    order,
                                    order_signature,
                                    referrer_info,
                                    param,
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

        Ok(())
    }

    // Process the fastlane order & attempt to fill if possible
    pub async fn on_fastlane_order(&self, signed_order_info: SignedOrderInfo) -> JitResult<()> {
        log::info!("Signed order received");
        let taker_authority = &signed_order_info.taker_authority;
        let taker_pubkey =
            Wallet::derive_user_account(taker_authority, signed_order_info.taker_subaccount_id());
        let taker_stats_key = Wallet::derive_stats_account(taker_authority);

        let order = signed_order_info.order_params();
        if order.auction_duration.is_some_and(|d| d == 0) || order.auction_duration.is_none() {
            return Ok(());
        }

        let order_sig = self.get_order_signatures(
            &taker_pubkey.to_string(),
            signed_order_info.order_uuid_str(),
        );

        // TODO: is this necessary??
        if self.ongoing_auctions.contains_key(&order_sig) {
            return Ok(());
        }

        let (taker, taker_stats) = tokio::try_join!(
            self.drift_client.get_user_account(&taker_pubkey),
            self.drift_client.get_user_stats(taker_authority),
        )?;

        match order.market_type {
            MarketType::Spot => {
                log::warn!("spot market fastlane unimplemented");
                return Ok(());
            }
            MarketType::Perp => {
                if let Some(param) = self.perp_params.get(&order.market_index) {
                    let perp_market = self
                        .drift_client
                        .program_data()
                        .perp_market_config_by_index(order.market_index)
                        .unwrap();
                    let remaining = order.base_asset_amount;
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

                    let jitter = Arc::clone(&self.jitter);
                    let order_signature = order_sig.clone();
                    let referrer_info = ReferrerInfo::get_referrer_info(taker_stats);
                    let jit_ix_params = param.clone();

                    let ongoing_auction = tokio::spawn(async move {
                        let _ = jitter
                            .try_fastlane_fill(
                                &signed_order_info,
                                order_signature,
                                &JitTakerParams::new(
                                    taker_pubkey,
                                    taker_stats_key,
                                    taker,
                                    referrer_info,
                                ),
                                &jit_ix_params,
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
        Ok(())
    }

    // Helper functions
    pub fn update_perp_params(&self, market_index: u16, params: JitIxParams) {
        self.perp_params.insert(market_index, params);
    }

    pub fn update_spot_params(&self, market_index: u16, params: JitIxParams) {
        self.spot_params.insert(market_index, params);
    }

    pub fn set_exclusion_criteria(&self, exclusion_criteria: bool) {
        self.exclusion_criteria
            .store(exclusion_criteria, Ordering::Relaxed);
    }

    pub fn get_order_signatures<T: Display>(&self, taker_key: &str, order_id: T) -> String {
        format!("{}-{}", taker_key, order_id)
    }
}

/// Aggressive JIT making strategy
pub struct Shotgun {
    authority: Pubkey,
    pub jit_proxy_client: JitProxyClient,
}

/// Implementing the Sniper is left as an exercise for the reader.
/// Good luck!
pub struct Sniper {
    pub jit_proxy_client: JitProxyClient,
    pub slot_subscriber: SlotSubscriber,
}

#[async_trait]
impl JitterStrategy for Shotgun {
    async fn try_fill(
        &self,
        taker: User,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: String,
        referrer_info: Option<ReferrerInfo>,
        params: JitIxParams,
    ) -> JitResult<()> {
        log::info!("Trying to fill with Shotgun:");
        log_details(&order);

        for i in 0..order.auction_duration {
            let referrer_info = referrer_info.clone();
            log::info!("Trying to fill: {:?} -> Attempt: {}", &order_sig, i + 1);

            if params.max_position == 0 || params.min_position == 0 {
                log::warn!(
                    "min or max position is 0 -> min: {} max: {}",
                    params.min_position,
                    params.max_position
                );
                return Ok(());
            }

            let jit_ix_params = JitIxParams::new(
                params.max_position,
                params.min_position,
                params.bid,
                params.ask,
                params.price_type,
                None,
            );

            let jit_result = self
                .jit_proxy_client
                .jit(
                    order.order_id,
                    &JitTakerParams::new(taker_key, taker_stats_key, taker, referrer_info),
                    jit_ix_params,
                    &self.authority,
                    None,
                )
                .await;

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
                    match check_err(err, &order_sig) {
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
    async fn try_fastlane_fill(
        &self,
        signed_order_info: &SignedOrderInfo,
        order_sig: String,
        taker_params: &JitTakerParams,
        jit_ix_params: &JitIxParams,
    ) -> JitResult<()> {
        log::info!("Trying to fill with Shotgun:");
        //  TODO: log the order deets
        // log_details(&order);

        let auction_duration = signed_order_info
            .order_params()
            .auction_duration
            .expect("auction duration set");
        for i in 0..auction_duration {
            log::info!("Trying to fill: {:?} -> Attempt: {}", &order_sig, i + 1);

            if jit_ix_params.max_position == 0 || jit_ix_params.min_position == 0 {
                log::warn!(
                    "min or max position is 0 -> min: {} max: {}",
                    jit_ix_params.min_position,
                    jit_ix_params.max_position
                );
                return Ok(());
            }

            let jit_result = self
                .jit_proxy_client
                .try_fastlane_fill(
                    signed_order_info,
                    taker_params,
                    jit_ix_params,
                    &self.authority,
                    None,
                )
                .await;

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
                    match check_err(err, &order_sig) {
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
