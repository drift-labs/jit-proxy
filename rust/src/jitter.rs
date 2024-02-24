use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use dashmap::DashMap;
use drift::state::user::{OrderStatus, User, UserStats};
use drift_sdk::{
    auction_subscriber::{AuctionSubscriber, AuctionSubscriberConfig}, constants::PROGRAM_ID as drift_program, event_emitter::Event, slot_subscriber::SlotSubscriber, types::{
        CommitmentConfig, 
        MarketType, 
        Order, 
        PerpMarket, ReferrerInfo
    }, websocket_program_account_subscriber::ProgramAccountUpdate, AccountProvider, DriftClient, Pubkey, Wallet
};
use jit_proxy::state::PriceType;
use solana_sdk::signature::Signature;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;

use crate::jit_proxy_client::{self, JitIxParams, JitProxyClient};
use crate::JitResult;


type UserFilter = dyn Fn(&User, &String, Order) -> bool + Send + Sync;

fn log_details(order: &Order) {
    log::info!("Market Type: {:?}", order.market_type);  
    log::info!("Market index: {}", order.market_index);
    log::info!("Order price: {}", order.price);
    log::info!("Order direction: {:?}", order.direction);
    log::info!("Auction start price: {}", order.auction_start_price);
    log::info!("Auction end price: {}", order.auction_end_price); 
    log::info!("Auction duration: {} slots", order.auction_duration);
    log::info!("Order base asset amount: {}", order.base_asset_amount);
    log::info!("Order base asset amount filled: {}", order.base_asset_amount_filled);
}

#[inline(always)]
fn check_err(err: String, order_sig: String) -> Option<u8> {
    if err.contains("0x1770") || err.contains("0x1771") {
        log::error!("Order: {} does not cross params yet, retrying", order_sig);
        None
    } else if err.contains("0x1779") {
        log::error!("Order: {} could not fill, retrying", order_sig);
        None
    } else if err.contains("0x1793") {
        log::error!("Oracle invalid, retrying");
        None
    } else if err.contains("0x1772") {
        log::error!("Order: {} already filled", order_sig);
        Some(1)
    } else {
        log::error!("Error: {}", err);
        Some(2)
    }       
}

#[derive(Clone)]
pub struct JitParams {
    bid: i64,
    ask: i64,
    min_position: i64,
    max_position: i64,
    price_type: PriceType,
    sub_account_id: Option<u16>,
}

impl JitParams {
    pub fn new(
        bid: i64,
        ask: i64,
        min_position: i64,
        max_position: i64,
        price_type: PriceType,
        sub_account_id: Option<u16>,
    ) -> Self {
        Self {
            bid,
            ask,
            min_position,
            max_position,
            price_type,
            sub_account_id,
        }
    }
}

pub struct Jitter<T: AccountProvider> {
    pub drift_client: DriftClient<T>,
    pub perp_params: DashMap<u16, JitParams>,
    pub spot_params: DashMap<u16, JitParams>,
    pub ongoing_auctions: DashMap<String, JoinHandle<()>>,
    pub user_filter: Option<Box<UserFilter>>,
    pub auction_sender: Sender<Box<dyn Event>>,
    pub jitter: Arc<dyn JitterStrategy + Send + Sync>,
}

#[async_trait]
pub trait JitterStrategy {
    async fn try_fill(&self, taker: User, taker_key: Pubkey, taker_stats_key: Pubkey, order: Order, order_sig: String, referrer_info: Option<ReferrerInfo>, params: JitParams) -> JitResult<()>;
}

impl<T: AccountProvider> Jitter<T> {
    pub fn new(
        drift_client: DriftClient<T>,
        jitter: Arc<dyn JitterStrategy + Send + Sync>, 
    ) -> Arc<Self> {

        let (auction_sender, mut auction_receiver): (Sender<Box<dyn Event>>, Receiver<Box<dyn Event>>) = mpsc::channel(100); 

        let jitter = Arc::new(Jitter {
            drift_client,
            perp_params: DashMap::new(),
            spot_params: DashMap::new(),
            ongoing_auctions: DashMap::new(),
            user_filter: None,
            auction_sender,
            jitter,
        });

        let jitter_clone = jitter.clone();
        // Set up a task to monitor for auction events & bounce them to the trampoline
        tokio::spawn(async move {
            while let Some(event) = auction_receiver.recv().await {
                let _ = jitter_clone.on_auction_sync(event);
            }
        });

        jitter
    }

    // Subscribe to auction events and start listening for them
    pub async fn subscribe(self: Arc<Self>, url: String) -> JitResult<()>{

        let auction_subscriber_config = AuctionSubscriberConfig {
            commitment: CommitmentConfig::processed(),
            resub_timeout_ms: None,
            url,
        };

        let mut auction_subscriber = AuctionSubscriber::new(auction_subscriber_config);
        auction_subscriber.subscribe().await?;

        let auction_sender = self.auction_sender.clone();
        auction_subscriber.event_emitter.subscribe("auction",  move |event| {
            let _ = auction_sender.try_send(event);
        });

        futures::future::pending::<()>().await;  // Keep the event loop alive forever
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

            let user_pubkey = auction.pubkey.clone();
            let user = auction.data_and_slot.data.clone();
            let slot = auction.data_and_slot.slot;
            let user_stats_key = Wallet::derive_stats_account(&Pubkey::from_str(&user_pubkey).unwrap(), &drift_program);

            for order in user.orders {  
                if order.status != OrderStatus::Open {
                    continue;
                }

                if !order.has_auction_price(order.slot, order.auction_duration, slot)? {
                    continue;
                }

                if let Some(user_filter) = &self.user_filter {
                    if user_filter(&user, &user_pubkey, order.clone()) {
                        continue;
                    }
                }

                let order_sig = self.get_order_signatures(&user_pubkey, order.order_id);

                if let Some(_) = self.ongoing_auctions.get(&order_sig) {
                    continue;
                }

                match order.market_type {
                    MarketType::Perp => {
                        log::info!("Perp Auction");

                        if let Some(_) = self.perp_params.get(&order.market_index) {

                            let perp_market: PerpMarket = self.drift_client.get_perp_market_info(order.market_index).await?;
                            
                            if order.base_asset_amount - order.base_asset_amount_filled <= perp_market.amm.min_order_size {
                                log::warn!("Order filled within min order size");
                                log::warn!("Remaining: {}", order.base_asset_amount - order.base_asset_amount_filled);
                                log::warn!("Minimum order size: {}", perp_market.amm.min_order_size);
                                return Ok(())
                            }

                            let jitter = self.jitter.clone();
                            let taker = user_pubkey.clone();
                            let order_signature = order_sig.clone();

                            let taker_stats: UserStats = self.drift_client.get_user_stats(&user_stats_key).await?;
                            let referrer_info = ReferrerInfo::get_referrer_info(taker_stats);

                            let perp_params = self.perp_params.clone();
                            let ongoing_auction = tokio::spawn(async move {
                                if let Some(param) = perp_params.get(&order.market_index) {
                                    let _ = jitter.try_fill(
                                        user.clone(), 
                                        Pubkey::from_str(&taker).unwrap(), 
                                        user_stats_key.clone(), 
                                        order.clone(), 
                                        order_signature.clone(),
                                        referrer_info,
                                        param.clone()
                                    ).await;
                                };
                            });

                            self.ongoing_auctions.insert(order_sig, ongoing_auction);
                        } else {
                            log::warn!("Jitter not listening to {}", order.market_index);
                            return Ok(())
                        }
                    }
                    MarketType::Spot => {
                        if let Some(_) = self.spot_params.get(&order.market_index) {
                            log::info!("Spot Auction");

                            let spot_market = self.drift_client.get_spot_market_info(order.market_index).await?;

                            if order.base_asset_amount - order.base_asset_amount_filled <= spot_market.min_order_size {
                                log::warn!("Order filled within min order size");
                                log::warn!("Remaining: {}", order.base_asset_amount - order.base_asset_amount_filled);
                                log::warn!("Minimum order size: {}", spot_market.min_order_size);
                                return Ok(())
                            }

                            let jitter = self.jitter.clone();
                            let taker = user_pubkey.clone();
                            let order_signature = order_sig.clone();

                            let taker_stats: UserStats = self.drift_client.get_user_stats(&user_stats_key).await?;
                            let referrer_info = ReferrerInfo::get_referrer_info(taker_stats);

                            let spot_params = self.spot_params.clone();
                            let ongoing_auction = tokio::spawn(async move {
                                if let Some(param) = spot_params.get(&order.market_index) {
                                    let _ = jitter.try_fill(
                                        user.clone(), 
                                        Pubkey::from_str(&taker).unwrap(), 
                                        user_stats_key.clone(), 
                                        order.clone(), 
                                        order_signature.clone(),
                                        referrer_info,
                                        param.clone()
                                    ).await;
                                };
                            });

                            self.ongoing_auctions.insert(order_sig, ongoing_auction);
                        } else {
                            log::warn!("Jitter not listening to {}", order.market_index);
                            return Ok(())
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

    pub fn set_user_filter(&mut self, user_filter: Box<UserFilter>) {
        self.user_filter = Some(user_filter);
    }

    pub fn get_order_signatures(&self, taker_key: &str, order_id: u32) -> String {
        format!("{}-{}", taker_key, order_id)
    }
}

pub struct Shotgun<T: AccountProvider> {
    pub jit_proxy_client: JitProxyClient<T>
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
    ) -> JitResult<()> {
        log::info!("Trying to fill with Shotgun:");
        log_details(&order);
        
        for i in 0..order.auction_duration {
            let referrer_info = referrer_info.clone();
            log::info!("Trying to fill: {:?} -> Attempt: {}", order_sig.clone(), i + 1);

            if params.max_position == 0 || params.min_position == 0 {
                log::warn!("min or max position is 0 -> min: {} max: {}", params.min_position, params.max_position);
                return Ok(())
            }

            let jit_ix_params =  JitIxParams::new(
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

            let jit_result = self.jit_proxy_client.jit(jit_ix_params).await;

            match jit_result {
                Ok(sig) => {
                    if sig == Signature::default() {
                        log::warn!("Failed to find order in taker orders");
                        continue;
                    }
                    log::info!("Order filled");
                    log::info!("Signature: {:?}", sig);
                    return Ok(())
                },
                Err(e) => {
                    let err = e.to_string();
                    match check_err(err, order_sig.clone()) {
                        Some(_) => return Ok(()),
                        None => continue,
                    
                    }
                }
            }

        }
        Ok(())
    }
}