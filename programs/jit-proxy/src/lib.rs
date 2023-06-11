use anchor_lang::prelude::*;
use drift::controller::position::PositionDirection;
use drift::instructions::{OrderParams};
use drift::state::state::State;
use drift::state::user::{User, UserStats};
use drift::cpi::accounts::{PlaceAndMake};
use drift::program::Drift;
use borsh::{BorshDeserialize, BorshSerialize};
use drift::instructions::optional_accounts::{AccountMaps, load_maps};
use drift::instructions::PostOnlyParam;
use drift::math::safe_math::SafeMath;
use drift::state::perp_market_map::{get_writable_perp_market_set, MarketSet};
use drift::state::user::{MarketType, OrderTriggerCondition, OrderType};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod jit_proxy {
    use super::*;

    pub fn jit_perp<'info>(ctx: Context<'_, '_, '_, 'info, Jit<'info>>, params: JitParams) -> Result<()> {
        let clock = Clock::get()?;
        let slot = clock.slot;

        let taker = ctx.accounts.taker.load()?;
        let maker = ctx.accounts.user.load()?;

        let taker_order = taker.get_order(params.taker_order_id).ok_or(ErrorCode::TakerOrderNotFound)?;
        let market_index = taker_order.market_index;
        let taker_direction = taker_order.direction;

        let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
        let AccountMaps {
            perp_market_map,
            mut oracle_map,
            ..
        } = load_maps(
            remaining_accounts_iter,
            &get_writable_perp_market_set(market_index),
            &MarketSet::new(),
            slot,
            None,
        )?;

        let perp_market = perp_market_map.get_ref(&market_index)?;
        let oracle_price = oracle_map.get_price_data(&perp_market.amm.oracle)?.price;

        let taker_price = taker_order.force_get_limit_price(Some(oracle_price), None, slot, perp_market.amm.order_tick_size)?;

        let maker_direction = taker_direction.opposite();
        if maker_direction == PositionDirection::Long {
            if taker_price > params.worst_price {
                msg!("taker price {} > worst price {}", taker_price, params.worst_price);
                return Err(ErrorCode::WorstPriceExceeded.into());
            }
        } else {
            if taker_price < params.worst_price {
                msg!("taker price {} < worst price {}", taker_price, params.worst_price);
                return Err(ErrorCode::WorstPriceExceeded.into());
            }
        }
        let maker_price = taker_price;

        let taker_base_asset_amount_unfilled = taker_order.get_base_asset_amount_unfilled(None)?;
        let maker_existing_position = maker.get_perp_position(market_index).map_or(0, |p| p.base_asset_amount);
        let maker_base_asset_amount = if maker_direction == PositionDirection::Long {
            let size = params.max_position.safe_sub(maker_existing_position)?;

            if size <= 0 {
                msg!("maker existing position {} >= max position {}", maker_existing_position, params.max_position);
            }

            size.unsigned_abs().min(taker_base_asset_amount_unfilled)
        } else {
            let size = maker_existing_position.safe_sub(params.max_position)?;

            if size <= 0 {
                msg!("maker existing position {} <= max position {}", maker_existing_position, params.max_position);
            }

            size.unsigned_abs().min(taker_base_asset_amount_unfilled)
        };

        let order_params = OrderParams {
            order_type: OrderType::Limit,
            market_type: MarketType::Perp,
            direction: maker_direction,
            user_order_id: 0,
            base_asset_amount: maker_base_asset_amount,
            price: maker_price,
            market_index,
            reduce_only: false,
            post_only: PostOnlyParam::MustPostOnly,
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

#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct JitParams {
    pub taker_order_id: u32,
    pub max_position: i64,
    pub worst_price: u64,
}

#[error_code]
#[derive(PartialEq, Eq)]
pub enum ErrorCode {
    #[msg("WorstPriceExceeded")]
    WorstPriceExceeded,
    #[msg("TakerOrderNotFound")]
    TakerOrderNotFound,
}

fn place_and_make<'info>(ctx: Context<'_, '_, '_, 'info, Jit<'info>>, taker_order_id: u32, order_params: OrderParams) -> Result<()> {
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

    drift::cpi::place_and_make_perp_order(cpi_context, order_params, taker_order_id)?;

    Ok(())
}