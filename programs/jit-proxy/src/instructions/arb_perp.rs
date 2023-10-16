use anchor_lang::prelude::*;
use drift::controller::position::PositionDirection;
use drift::cpi::accounts::PlaceAndTake;
use drift::instructions::optional_accounts::{load_maps, AccountMaps};
use drift::instructions::{OrderParams, PostOnlyParam};
use drift::program::Drift;
use drift::state::perp_market_map::MarketSet;

use drift::math::orders::find_bids_and_asks_from_users;
use drift::state::state::State;
use drift::state::user::{MarketType, OrderTriggerCondition, OrderType, User, UserStats};
use drift::state::user_map::load_user_maps;

use crate::error::ErrorCode;

pub fn arb_perp<'info>(
    ctx: Context<'_, '_, '_, 'info, ArbPerp<'info>>,
    market_index: u16,
) -> Result<()> {
    let clock = Clock::get()?;
    let slot = clock.slot;
    let now = clock.unix_timestamp;

    let taker = ctx.accounts.user.load()?;

    let quote_init = taker
        .get_perp_position(market_index)
        .map_or(0, |p| p.quote_asset_amount);

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        perp_market_map,
        mut oracle_map,
        ..
    } = load_maps(
        remaining_accounts_iter,
        &MarketSet::new(),
        &MarketSet::new(),
        slot,
        None,
    )?;

    let (makers, _) = load_user_maps(remaining_accounts_iter, true)?;

    let perp_market = perp_market_map.get_ref(&market_index)?;
    let oracle_price_data = oracle_map.get_price_data(&perp_market.amm.oracle)?;

    let (bids, asks) =
        find_bids_and_asks_from_users(&perp_market, oracle_price_data, &makers, slot, now)?;

    let best_bid = bids.first().ok_or(ErrorCode::NoBestBid)?;
    let best_ask = asks.first().ok_or(ErrorCode::NoBestAsk)?;

    if best_bid.price < best_ask.price {
        return Err(ErrorCode::NoArbOpportunity.into());
    }

    let base_asset_amount = best_bid.base_asset_amount.min(best_ask.base_asset_amount);

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
            immediate_or_cancel: true,
            max_ts: None,
            trigger_price: None,
            trigger_condition: OrderTriggerCondition::Above,
            oracle_price_offset: None,
            auction_duration: None,
            auction_start_price: None,
            auction_end_price: None,
        }
    };

    let order_params = vec![
        get_order_params(PositionDirection::Long, best_ask.price),
        get_order_params(PositionDirection::Short, best_bid.price),
    ];

    drop(taker);
    drop(perp_market);

    place_and_take(&ctx, order_params)?;

    let taker = ctx.accounts.user.load()?;
    let quote_end = taker
        .get_perp_position(market_index)
        .map_or(0, |p| p.quote_asset_amount);

    if quote_end <= quote_init {
        return Err(ErrorCode::NoArbOpportunity.into());
    }

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
