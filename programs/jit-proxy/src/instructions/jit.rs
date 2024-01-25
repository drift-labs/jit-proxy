use anchor_lang::prelude::*;
use drift::controller::position::PositionDirection;
use drift::cpi::accounts::PlaceAndMake;
use drift::error::DriftResult;
use drift::instructions::optional_accounts::{load_maps, AccountMaps};
use drift::math::casting::Cast;
use drift::math::safe_math::SafeMath;
use drift::program::Drift;
use drift::state::order_params::OrderParams;
use drift::state::perp_market_map::MarketSet;
use drift::state::state::State;
use drift::state::user::{MarketType as DriftMarketType, OrderTriggerCondition, OrderType};
use drift::state::user::{User, UserStats};

use crate::error::ErrorCode;
use crate::state::{PostOnlyParam, PriceType};

pub fn jit<'info>(ctx: Context<'_, '_, '_, 'info, Jit<'info>>, params: JitParams) -> Result<()> {
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

    let slots_left = taker_order
        .slot
        .safe_add(taker_order.auction_duration.cast()?)?
        .cast::<i64>()?
        .safe_sub(slot.cast()?)?;
    msg!(
        "slot = {} auction duration = {} slots_left = {}",
        slot,
        taker_order.auction_duration,
        slots_left
    );

    if taker_order.order_type == OrderType::Oracle {
        msg!(
            "order type {:?} auction start {} auction end {} oracle price offset {}",
            taker_order.order_type,
            taker_order.auction_start_price,
            taker_order.auction_end_price,
            taker_order.oracle_price_offset
        );
    } else {
        msg!(
            "order type {:?} auction start {} auction end {} limit price {}",
            taker_order.order_type,
            taker_order.auction_start_price,
            taker_order.auction_end_price,
            taker_order.price
        );
    }

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

    let (oracle_price, tick_size, min_order_size) = if market_type == DriftMarketType::Perp {
        let perp_market = perp_market_map.get_ref(&market_index)?;
        let oracle_price = oracle_map.get_price_data(&perp_market.amm.oracle)?.price;

        (
            oracle_price,
            perp_market.amm.order_tick_size,
            perp_market.amm.min_order_size,
        )
    } else {
        let spot_market = spot_market_map.get_ref(&market_index)?;
        let oracle_price = oracle_map.get_price_data(&spot_market.oracle)?.price;

        (
            oracle_price,
            spot_market.order_tick_size,
            spot_market.min_order_size,
        )
    };

    let taker_price =
        match taker_order.get_limit_price(Some(oracle_price), None, slot, tick_size)? {
            Some(price) => price,
            None if market_type == DriftMarketType::Perp => {
                msg!("taker order didnt have price. deriving fallback");
                // if the order doesn't have a price, drift users amm price for taker price
                let perp_market = perp_market_map.get_ref(&market_index)?;
                let reserve_price = perp_market.amm.reserve_price()?;
                match taker_direction {
                    PositionDirection::Long => perp_market.amm.ask_price(reserve_price)?,
                    PositionDirection::Short => perp_market.amm.bid_price(reserve_price)?,
                }
            }
            None => {
                // Shouldnt be possible for spot
                msg!("taker order didnt have price");
                return Err(ErrorCode::TakerOrderNotFound.into());
            }
        };

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

    let taker_base_asset_amount_unfilled = taker_order
        .get_base_asset_amount_unfilled(None)?
        .max(min_order_size);
    let maker_existing_position = if market_type == DriftMarketType::Perp {
        let perp_market = perp_market_map.get_ref(&market_index)?;
        let perp_position = maker.get_perp_position(market_index);
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
        maker
            .get_spot_position(market_index)
            .map_or(0, |p| p.get_signed_token_amount(&spot_market).unwrap())
            .cast::<i64>()?
    };

    let maker_base_asset_amount = match check_position_limits(
        params,
        maker_direction,
        taker_base_asset_amount_unfilled,
        maker_existing_position,
        min_order_size,
    ) {
        Ok(size) => size,
        Err(e) => {
            return Err(e);
        }
    };

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

    place_and_make(&ctx, params.taker_order_id, order_params)?;

    let taker = ctx.accounts.taker.load()?;

    let taker_base_asset_amount_unfilled_after = match taker.get_order(params.taker_order_id) {
        Some(order) => order.get_base_asset_amount_unfilled(None)?,
        None => 0,
    };

    if taker_base_asset_amount_unfilled_after == taker_base_asset_amount_unfilled {
        // taker order failed to fill
        msg!(
            "taker price = {} oracle price = {}",
            taker_price,
            oracle_price
        );
        msg!("jit params {:?}", params);
        if market_type == DriftMarketType::Perp {
            let perp_market = perp_market_map.get_ref(&market_index)?;
            let reserve_price = perp_market.amm.reserve_price()?;
            let (bid_price, ask_price) = perp_market.amm.bid_ask_price(reserve_price)?;
            msg!(
                "vamm bid price = {} vamm ask price = {}",
                bid_price,
                ask_price
            );
        }
        return Err(ErrorCode::NoFill.into());
    }

    Ok(())
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

impl Default for JitParams {
    fn default() -> Self {
        Self {
            taker_order_id: 0,
            max_position: 0,
            min_position: 0,
            bid: 0,
            ask: 0,
            price_type: PriceType::Limit,
            post_only: None,
        }
    }
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

fn check_position_limits(
    params: JitParams,
    maker_direction: PositionDirection,
    taker_base_asset_amount_unfilled: u64,
    maker_existing_position: i64,
    min_order_size: u64,
) -> Result<u64> {
    if maker_direction == PositionDirection::Long {
        let size = params.max_position.safe_sub(maker_existing_position)?;

        if size <= min_order_size.cast()? {
            msg!(
                "maker existing position {} >= max position {} + min order size {}",
                maker_existing_position,
                params.max_position,
                min_order_size
            );
            return Err(ErrorCode::PositionLimitBreached.into());
        }

        Ok(size.unsigned_abs().min(taker_base_asset_amount_unfilled))
    } else {
        let size = maker_existing_position.safe_sub(params.min_position)?;

        if size <= min_order_size.cast()? {
            msg!(
                "maker existing position {} <= min position {} + min order size {}",
                maker_existing_position,
                params.min_position,
                min_order_size
            );
            return Err(ErrorCode::PositionLimitBreached.into());
        }

        Ok(size.unsigned_abs().min(taker_base_asset_amount_unfilled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_position_limits() {
        let params = JitParams {
            max_position: 100,
            min_position: -100,
            ..Default::default()
        };

        // same direction, doesn't breach
        let result = check_position_limits(params, PositionDirection::Long, 10, 40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);
        let result = check_position_limits(params, PositionDirection::Short, 10, -40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);

        // same direction, whole order breaches, only takes enough to hit limit
        let result = check_position_limits(params, PositionDirection::Long, 100, 40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 60);
        let result = check_position_limits(params, PositionDirection::Short, 100, -40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 60);

        // opposite direction, doesn't breach
        let result = check_position_limits(params, PositionDirection::Long, 10, -40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);
        let result = check_position_limits(params, PositionDirection::Short, 10, 40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);

        // opposite direction, whole order breaches, only takes enough to take flipped limit
        let result = check_position_limits(params, PositionDirection::Long, 200, -40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 140);
        let result = check_position_limits(params, PositionDirection::Short, 200, 40, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 140);

        // opposite direction, maker already breached, allows reducing
        let result = check_position_limits(params, PositionDirection::Long, 200, -150, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 200);
        let result = check_position_limits(params, PositionDirection::Short, 200, 150, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 200);

        // same direction, maker already breached, errors
        let result = check_position_limits(params, PositionDirection::Long, 200, 150, 0);
        assert!(result.is_err());
        let result = check_position_limits(params, PositionDirection::Short, 200, -150, 0);
        assert!(result.is_err());
    }
}

fn place_and_make<'info>(
    ctx: &Context<'_, '_, '_, 'info, Jit<'info>>,
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
