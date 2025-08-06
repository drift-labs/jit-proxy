use anchor_lang::prelude::*;
use drift::controller::position::PositionDirection;
use drift::cpi::accounts::PlaceAndTake;
use drift::error::DriftResult;
use drift::instructions::optional_accounts::{load_maps, AccountMaps};
use drift::math::casting::Cast;
use drift::math::constants::{BASE_PRECISION, MARGIN_PRECISION_U128, QUOTE_PRECISION};
use drift::math::margin::MarginRequirementType;
use drift::program::Drift;
use drift::state::order_params::{OrderParams, OrderParamsBitFlag, PostOnlyParam};
use std::collections::BTreeSet;
use std::ops::Deref;

use drift::math::orders::find_bids_and_asks_from_users;
use drift::math::safe_math::SafeMath;
use drift::state::oracle::OraclePriceData;
use drift::state::state::State;
use drift::state::user::{MarketType, OrderTriggerCondition, OrderType, User, UserStats};
use drift::state::user_map::load_user_maps;

use crate::error::ErrorCode;

pub fn arb_perp<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, ArbPerp<'info>>,
    market_index: u16,
) -> Result<()> {
    let clock = Clock::get()?;
    let slot = clock.slot;
    let now = clock.unix_timestamp;

    let taker = ctx.accounts.user.load()?;

    let (base_init, quote_init) = taker
        .get_perp_position(market_index)
        .map_or((0, 0), |p| (p.base_asset_amount, p.quote_asset_amount));

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        perp_market_map,
        mut oracle_map,
        spot_market_map,
    } = load_maps(
        remaining_accounts_iter,
        &BTreeSet::new(),
        &BTreeSet::new(),
        slot,
        None,
    )?;

    let quote_asset_token_amount = taker
        .get_quote_spot_position()
        .get_token_amount(spot_market_map.get_quote_spot_market()?.deref())?;

    let (makers, _) = load_user_maps(remaining_accounts_iter, true)?;

    let perp_market = perp_market_map.get_ref(&market_index)?;
    let oracle_price_data = oracle_map.get_price_data(&perp_market.oracle_id())?;

    let (bids, asks) =
        find_bids_and_asks_from_users(&perp_market, oracle_price_data, &makers, slot, now)?;

    let best_bid = bids.first().ok_or(ErrorCode::NoBestBid)?;
    let best_ask = asks.first().ok_or(ErrorCode::NoBestAsk)?;

    if best_bid.price < best_ask.price {
        return Err(ErrorCode::NoArbOpportunity.into());
    }

    let base_asset_amount = best_bid.base_asset_amount.min(best_ask.base_asset_amount);

    let (intermediate_base, start_direction) = if base_init >= 0 {
        let intermediate_base = base_init.safe_sub(base_asset_amount.cast()?)?;
        (intermediate_base, PositionDirection::Short)
    } else {
        let intermediate_base = base_init.safe_add(base_asset_amount.cast()?)?;
        (intermediate_base, PositionDirection::Long)
    };

    let init_margin_ratio = perp_market.get_margin_ratio(
        intermediate_base.unsigned_abs().cast()?,
        MarginRequirementType::Initial,
        taker.is_high_leverage_mode(MarginRequirementType::Initial),
    )?;

    // assumes all free collateral in quote asset token
    let max_base_asset_amount = calculate_max_base_asset_amount(
        quote_asset_token_amount,
        init_margin_ratio,
        oracle_price_data,
    )?;

    let base_asset_amount = base_asset_amount
        .min(max_base_asset_amount.cast()?)
        .max(perp_market.amm.min_order_size);

    let get_order_params = |taker_direction: PositionDirection, taker_price: u64| -> OrderParams {
        OrderParams {
            order_type: OrderType::Limit,
            market_type: MarketType::Perp,
            direction: taker_direction,
            user_order_id: 0,
            base_asset_amount,
            price: taker_price,
            market_index,
            reduce_only: false,
            post_only: PostOnlyParam::None,
            bit_flags: OrderParamsBitFlag::ImmediateOrCancel as u8,
            max_ts: None,
            trigger_price: None,
            trigger_condition: OrderTriggerCondition::Above,
            oracle_price_offset: None,
            auction_duration: None,
            auction_start_price: None,
            auction_end_price: None,
        }
    };

    let order_params = if start_direction == PositionDirection::Long {
        vec![
            get_order_params(PositionDirection::Long, best_ask.price),
            get_order_params(PositionDirection::Short, best_bid.price),
        ]
    } else {
        vec![
            get_order_params(PositionDirection::Short, best_bid.price),
            get_order_params(PositionDirection::Long, best_ask.price),
        ]
    };

    drop(taker);
    drop(perp_market);

    place_and_take(&ctx, order_params)?;

    let taker = ctx.accounts.user.load()?;
    let (base_end, quote_end) = taker
        .get_perp_position(market_index)
        .map_or((0, 0), |p| (p.base_asset_amount, p.quote_asset_amount));

    if base_end != base_init || quote_end <= quote_init {
        msg!(
            "base_end {} base_init {} quote_end {} quote_init {}",
            base_end,
            base_init,
            quote_end,
            quote_init
        );
        return Err(ErrorCode::NoArbOpportunity.into());
    }

    msg!("pnl {}", quote_end - quote_init);

    Ok(())
}

#[derive(Accounts)]
pub struct ArbPerp<'info> {
    pub state: Box<Account<'info, State>>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    #[account(mut)]
    pub user_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
    pub drift_program: Program<'info, Drift>,
}

fn calculate_max_base_asset_amount(
    quote_asset_token_amount: u128,
    init_margin_ratio: u32,
    oracle_price_data: &OraclePriceData,
) -> DriftResult<u128> {
    quote_asset_token_amount
        .saturating_sub((quote_asset_token_amount / 100).min(10 * QUOTE_PRECISION)) // room for error
        .safe_mul(MARGIN_PRECISION_U128)?
        .safe_div(init_margin_ratio.cast()?)?
        .safe_mul(BASE_PRECISION)?
        .safe_div(oracle_price_data.price.cast()?)
}

fn place_and_take<'info>(
    ctx: &Context<'_, '_, '_, 'info, ArbPerp<'info>>,
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

        drift::cpi::place_and_take_perp_order(cpi_context, order_params, None)?;
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use drift::math::constants::{MARGIN_PRECISION, PRICE_PRECISION_I64, QUOTE_PRECISION};
    use drift::state::oracle::OraclePriceData;

    #[test]
    pub fn calculate_max_base_asset_amount() {
        let quote_asset_token_amount = 100 * QUOTE_PRECISION;
        let init_margin_ratio = MARGIN_PRECISION / 10;
        let oracle_price_data = OraclePriceData {
            price: 100 * PRICE_PRECISION_I64,
            ..OraclePriceData::default()
        };

        let max_base_asset_amount = super::calculate_max_base_asset_amount(
            quote_asset_token_amount,
            init_margin_ratio,
            &oracle_price_data,
        )
        .unwrap();

        assert_eq!(max_base_asset_amount, 9900000000);
    }
}
