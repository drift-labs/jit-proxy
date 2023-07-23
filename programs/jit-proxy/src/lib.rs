use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};
use drift::controller::position::PositionDirection;
use drift::cpi::accounts::{PlaceAndMake, PlaceAndTake};
use drift::error::DriftResult;
use drift::instructions::optional_accounts::{load_maps, AccountMaps};
use drift::instructions::OrderParams;
use drift::instructions::PostOnlyParam as DriftPostOnlyParam;
use drift::math::casting::Cast;
use drift::math::matching::do_orders_cross;
use drift::math::orders::find_maker_orders;
use drift::math::safe_math::SafeMath;
use drift::program::Drift;
use drift::state::oracle_map::OracleMap;
use drift::state::perp_market_map::{MarketSet, PerpMarketMap};
use drift::state::spot_market_map::SpotMarketMap;
use drift::state::state::State;
use drift::state::user::{MarketType as DriftMarketType, OrderTriggerCondition, OrderType};
use drift::state::user::{User, UserStats};
use drift::state::user_map::load_user_maps;

declare_id!("J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP");

#[program]
pub mod jit_proxy {
    use super::*;
    use std::cell::Ref;

    pub fn jit<'info>(
        ctx: Context<'_, '_, '_, 'info, Jit<'info>>,
        params: JitParams,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let slot = clock.slot;

        let taker = ctx.accounts.taker.load()?;
        let maker = ctx.accounts.user.load()?;

        let taker_order = taker
            .get_order(params.taker_order_id)
            .ok_or(ErrorCode::TakerOrderNotFound)?;
        let market_type = taker_order.market_type;
        let market_index = taker_order.market_index;
        let taker_direction = taker_order.direction;

        let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
        let AccountMaps {
            perp_market_map,
            spot_market_map,
            mut oracle_map,
        } = load_maps(
            remaining_accounts_iter,
            &MarketSet::new(),
            &MarketSet::new(),
            slot,
            None,
        )?;

        let (oracle_price, tick_size) = get_oracle_price_and_tick_size(
            market_index,
            market_type,
            &perp_market_map,
            &spot_market_map,
            &mut oracle_map,
        )?;

        let taker_price =
            taker_order.force_get_limit_price(Some(oracle_price), None, slot, tick_size)?;

        let maker_direction = taker_direction.opposite();
        let maker_worst_price = params.get_worst_price(oracle_price, taker_direction)?;
        match maker_direction {
            PositionDirection::Long => {
                if taker_price > maker_worst_price {
                    msg!(
                        "taker price {} > worst bid {}",
                        taker_price,
                        maker_worst_price
                    );
                    return Err(ErrorCode::BidNotCrossed.into());
                }
            }
            PositionDirection::Short => {
                if taker_price < maker_worst_price {
                    msg!(
                        "taker price {} < worst ask {}",
                        taker_price,
                        maker_worst_price
                    );
                    return Err(ErrorCode::AskNotCrossed.into());
                }
            }
        }
        let maker_price = taker_price;

        let taker_base_asset_amount_unfilled = taker_order.get_base_asset_amount_unfilled(None)?;
        let maker_base_asset_amount = calculate_max_order_size(
            &maker,
            maker_direction,
            Some(taker_base_asset_amount_unfilled),
            params.min_position,
            params.max_position,
            market_index,
            market_type,
            oracle_price,
            &perp_market_map,
            &spot_market_map,
        )?;

        let order_params = OrderParams {
            order_type: OrderType::Limit,
            market_type,
            direction: maker_direction,
            user_order_id: 0,
            base_asset_amount: maker_base_asset_amount,
            price: maker_price,
            market_index,
            reduce_only: false,
            post_only: params
                .post_only
                .unwrap_or(PostOnlyParam::MustPostOnly)
                .to_drift_param(),
            immediate_or_cancel: true,
            max_ts: None,
            trigger_price: None,
            trigger_condition: OrderTriggerCondition::Above,
            oracle_price_offset: None,
            auction_duration: None,
            auction_start_price: None,
            auction_end_price: None,
        };

        drop(taker);
        drop(maker);

        place_and_make(ctx, params.taker_order_id, order_params)?;

        Ok(())
    }

    pub fn take<'info>(
        ctx: Context<'_, '_, '_, 'info, Take<'info>>,
        params: TakeParams,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let slot = clock.slot;

        let taker = ctx.accounts.user.load()?;

        let market_index = params.market_index;
        let market_type = params.market_type.to_drift_param();

        let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
        let AccountMaps {
            perp_market_map,
            spot_market_map,
            mut oracle_map,
        } = load_maps(
            remaining_accounts_iter,
            &MarketSet::new(),
            &MarketSet::new(),
            slot,
            None,
        )?;

        let (makers, _) = load_user_maps(remaining_accounts_iter)?;

        let (oracle_price, tick_size) = get_oracle_price_and_tick_size(
            market_index,
            market_type,
            &perp_market_map,
            &spot_market_map,
            &mut oracle_map,
        )?;

        let can_take = |taker_direction: PositionDirection, taker_price: u64| -> Result<bool> {
            let mut best_maker = match taker_direction {
                PositionDirection::Long => 0_u64,
                PositionDirection::Short => u64::MAX,
            };

            let maker_direction = taker_direction.opposite();
            for (_, maker_account_loader) in makers.0.iter() {
                let maker: Ref<User> = maker_account_loader.load()?;

                let maker_order_price_and_indexes = find_maker_orders(
                    &maker,
                    &maker_direction,
                    &market_type,
                    market_index,
                    Some(oracle_price),
                    slot,
                    tick_size,
                )?;

                // check if taker crosses any maker orders
                for (_, maker_price) in maker_order_price_and_indexes {
                    if do_orders_cross(maker_direction, maker_price, taker_price) {
                        return Ok(true);
                    }

                    best_maker = match taker_direction {
                        PositionDirection::Long => best_maker.max(maker_price),
                        PositionDirection::Short => best_maker.min(maker_price),
                    };
                }
            }

            msg!(
                "taker price {:?} direction {}",
                taker_direction,
                taker_price
            );
            msg!("best maker price: {}", best_maker);

            Ok(false)
        };

        let bid = params.get_worst_price(oracle_price, PositionDirection::Long)?;
        let bid_crosses = can_take(PositionDirection::Long, bid)?;

        let ask = params.get_worst_price(oracle_price, PositionDirection::Short)?;
        let ask_crosses = can_take(PositionDirection::Short, ask)?;

        if !bid_crosses && !ask_crosses {
            msg!("did not cross any maker orders");
            return Err(ErrorCode::DidNotCrossMakers.into());
        }

        let get_order_params =
            |taker_direction: PositionDirection, taker_price: u64| -> Result<OrderParams> {
                let taker_base_asset_amount = calculate_max_order_size(
                    &taker,
                    taker_direction,
                    None,
                    params.min_position,
                    params.max_position,
                    market_index,
                    market_type,
                    oracle_price,
                    &perp_market_map,
                    &spot_market_map,
                )?;

                let order_params = OrderParams {
                    order_type: OrderType::Limit,
                    market_type,
                    direction: taker_direction,
                    user_order_id: 0,
                    base_asset_amount: taker_base_asset_amount,
                    price: taker_price,
                    market_index,
                    reduce_only: false,
                    post_only: PostOnlyParam::None.to_drift_param(),
                    immediate_or_cancel: true,
                    max_ts: None,
                    trigger_price: None,
                    trigger_condition: OrderTriggerCondition::Above,
                    oracle_price_offset: None,
                    auction_duration: None,
                    auction_start_price: None,
                    auction_end_price: None,
                };

                Ok(order_params)
            };

        let mut orders_params = Vec::with_capacity(2);

        if bid_crosses {
            msg!("Trying to buy. Bid {}", bid);
            orders_params.push(get_order_params(PositionDirection::Long, bid)?);
        }

        if ask_crosses {
            msg!("Trying to sell. Ask {}", ask);
            orders_params.push(get_order_params(PositionDirection::Short, ask)?);
        }

        drop(taker);

        place_and_take(ctx, orders_params)?;

        Ok(())
    }
}

fn get_oracle_price_and_tick_size(
    market_index: u16,
    market_type: DriftMarketType,
    perp_market_map: &PerpMarketMap,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> DriftResult<(i64, u64)> {
    let (oracle_price, tick_size) = if market_type == DriftMarketType::Perp {
        let perp_market = perp_market_map.get_ref(&market_index)?;
        let oracle_price = oracle_map.get_price_data(&perp_market.amm.oracle)?.price;

        (oracle_price, perp_market.amm.order_tick_size)
    } else {
        let spot_market = spot_market_map.get_ref(&market_index)?;
        let oracle_price = oracle_map.get_price_data(&spot_market.oracle)?.price;

        (oracle_price, spot_market.order_tick_size)
    };

    Ok((oracle_price, tick_size))
}

fn calculate_max_order_size(
    user: &User,
    direction: PositionDirection,
    counter_party_size: Option<u64>,
    min_position: i64,
    max_position: i64,
    market_index: u16,
    market_type: DriftMarketType,
    oracle_price: i64,
    perp_market_map: &PerpMarketMap,
    spot_market_map: &SpotMarketMap,
) -> DriftResult<u64> {
    let user_existing_position = if market_type == DriftMarketType::Perp {
        let perp_market = perp_market_map.get_ref(&market_index)?;
        let perp_position = user.get_perp_position(market_index);
        match perp_position {
            Ok(perp_position) => {
                perp_position
                    .simulate_settled_lp_position(&perp_market, oracle_price)?
                    .base_asset_amount
            }
            Err(_) => 0,
        }
    } else {
        let spot_market = spot_market_map.get_ref(&market_index)?;
        user.get_spot_position(market_index)
            .map_or(0, |p| p.get_signed_token_amount(&spot_market).unwrap())
            .cast::<i64>()?
    };

    let user_base_asset_amount = if direction == PositionDirection::Long {
        let size = max_position.safe_sub(user_existing_position)?;

        if size <= 0 {
            msg!(
                "user existing position {} >= max position {}",
                user_existing_position,
                max_position
            );
        }

        size.unsigned_abs()
            .min(counter_party_size.unwrap_or(u64::MAX))
    } else {
        let size = user_existing_position.safe_sub(min_position)?;

        if size <= 0 {
            msg!(
                "user existing position {} <= min position {}",
                user_existing_position,
                min_position
            );
        }

        size.unsigned_abs()
            .min(counter_party_size.unwrap_or(u64::MAX))
    };

    Ok(user_base_asset_amount)
}

#[derive(Accounts)]
pub struct Jit<'info> {
    pub state: Box<Account<'info, State>>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    #[account(mut)]
    pub user_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub taker: AccountLoader<'info, User>,
    #[account(mut)]
    pub taker_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
    pub drift_program: Program<'info, Drift>,
}

#[derive(Debug, Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Eq)]
pub struct JitParams {
    pub taker_order_id: u32,
    pub max_position: i64,
    pub min_position: i64,
    pub bid: i64,
    pub ask: i64,
    pub price_type: PriceType,
    pub post_only: Option<PostOnlyParam>,
}

impl JitParams {
    pub fn get_worst_price(
        self,
        oracle_price: i64,
        taker_direction: PositionDirection,
    ) -> DriftResult<u64> {
        match (taker_direction, self.price_type) {
            (PositionDirection::Long, PriceType::Limit) => Ok(self.ask.unsigned_abs()),
            (PositionDirection::Short, PriceType::Limit) => Ok(self.bid.unsigned_abs()),
            (PositionDirection::Long, PriceType::Oracle) => {
                Ok(oracle_price.safe_add(self.ask)?.unsigned_abs())
            }
            (PositionDirection::Short, PriceType::Oracle) => {
                Ok(oracle_price.safe_add(self.bid)?.unsigned_abs())
            }
        }
    }
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum PostOnlyParam {
    None,
    MustPostOnly, // Tx fails if order can't be post only
    TryPostOnly,  // Tx succeeds and order not placed if can't be post only
}

impl PostOnlyParam {
    pub fn to_drift_param(self) -> DriftPostOnlyParam {
        match self {
            PostOnlyParam::None => DriftPostOnlyParam::None,
            PostOnlyParam::MustPostOnly => DriftPostOnlyParam::MustPostOnly,
            PostOnlyParam::TryPostOnly => DriftPostOnlyParam::TryPostOnly,
        }
    }
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum PriceType {
    Limit,
    Oracle,
}

#[error_code]
#[derive(PartialEq, Eq)]
pub enum ErrorCode {
    #[msg("BidNotCrossed")]
    BidNotCrossed,
    #[msg("AskNotCrossed")]
    AskNotCrossed,
    #[msg("TakerOrderNotFound")]
    TakerOrderNotFound,
    #[msg("DidNotCrossMakers")]
    DidNotCrossMakers,
}

fn place_and_make<'info>(
    ctx: Context<'_, '_, '_, 'info, Jit<'info>>,
    taker_order_id: u32,
    order_params: OrderParams,
) -> Result<()> {
    let drift_program = ctx.accounts.drift_program.to_account_info().clone();
    let cpi_accounts = PlaceAndMake {
        state: ctx.accounts.state.to_account_info().clone(),
        user: ctx.accounts.user.to_account_info().clone(),
        user_stats: ctx.accounts.user_stats.to_account_info().clone(),
        authority: ctx.accounts.authority.to_account_info().clone(),
        taker: ctx.accounts.taker.to_account_info().clone(),
        taker_stats: ctx.accounts.taker_stats.to_account_info().clone(),
    };

    let cpi_context = CpiContext::new(drift_program, cpi_accounts)
        .with_remaining_accounts(ctx.remaining_accounts.into());

    if order_params.market_type == DriftMarketType::Perp {
        drift::cpi::place_and_make_perp_order(cpi_context, order_params, taker_order_id)?;
    } else {
        drift::cpi::place_and_make_spot_order(cpi_context, order_params, taker_order_id, None)?;
    }

    Ok(())
}

#[derive(Accounts)]
pub struct Take<'info> {
    pub state: Box<Account<'info, State>>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    #[account(mut)]
    pub user_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
    pub drift_program: Program<'info, Drift>,
}

#[derive(Debug, Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Eq)]
pub struct TakeParams {
    pub market_index: u16,
    pub market_type: MarketType,
    pub max_position: i64,
    pub min_position: i64,
    pub bid: i64,
    pub ask: i64,
    pub price_type: PriceType,
    pub fulfillment_method: Option<u8>,
}

impl TakeParams {
    pub fn get_worst_price(
        self,
        oracle_price: i64,
        taker_direction: PositionDirection,
    ) -> DriftResult<u64> {
        match (taker_direction, self.price_type) {
            (PositionDirection::Long, PriceType::Limit) => Ok(self.bid.unsigned_abs()),
            (PositionDirection::Short, PriceType::Limit) => Ok(self.ask.unsigned_abs()),
            (PositionDirection::Long, PriceType::Oracle) => {
                Ok(oracle_price.safe_add(self.bid)?.unsigned_abs())
            }
            (PositionDirection::Short, PriceType::Oracle) => {
                Ok(oracle_price.safe_add(self.ask)?.unsigned_abs())
            }
        }
    }
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum MarketType {
    Perp,
    Spot,
}

impl MarketType {
    pub fn to_drift_param(self) -> DriftMarketType {
        match self {
            MarketType::Perp => DriftMarketType::Perp,
            MarketType::Spot => DriftMarketType::Spot,
        }
    }
}

fn place_and_take<'info>(
    ctx: Context<'_, '_, '_, 'info, Take<'info>>,
    orders_params: Vec<OrderParams>,
) -> Result<()> {
    for order_params in orders_params {
        let drift_program = ctx.accounts.drift_program.to_account_info().clone();
        let cpi_accounts = PlaceAndTake {
            state: ctx.accounts.state.to_account_info().clone(),
            user: ctx.accounts.user.to_account_info().clone(),
            user_stats: ctx.accounts.user_stats.to_account_info().clone(),
            authority: ctx.accounts.authority.to_account_info().clone(),
        };

        let cpi_context = CpiContext::new(drift_program, cpi_accounts)
            .with_remaining_accounts(ctx.remaining_accounts.into());

        if order_params.market_type == DriftMarketType::Perp {
            drift::cpi::place_and_take_perp_order(cpi_context, order_params, None)?;
        } else {
            drift::cpi::place_and_take_spot_order(cpi_context, order_params, None, None)?;
        }
    }

    Ok(())
}
