use anchor_lang::prelude::*;
use drift::instructions::optional_accounts::{load_maps, AccountMaps};
use drift::math::casting::Cast;
use drift::math::safe_math::SafeMath;
use drift::state::user::User;
use std::collections::BTreeSet;

use crate::error::ErrorCode;
use crate::state::MarketType;

pub fn check_order_constraints<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CheckOrderConstraints<'info>>,
    constraints: Vec<OrderConstraint>,
) -> Result<()> {
    let clock = Clock::get()?;
    let slot = clock.slot;

    let user = ctx.accounts.user.load()?;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        perp_market_map: _,
        spot_market_map,
        oracle_map: _,
    } = load_maps(
        remaining_accounts_iter,
        &BTreeSet::new(),
        &BTreeSet::new(),
        slot,
        None,
    )?;

    for constraint in constraints.iter() {
        if constraint.market_type == MarketType::Spot {
            let spot_market = spot_market_map.get_ref(&constraint.market_index)?;
            let spot_position = match user.get_spot_position(constraint.market_index) {
                Ok(spot_position) => spot_position,
                Err(_) => continue,
            };

            let signed_token_amount = spot_position
                .get_signed_token_amount(&spot_market)?
                .cast::<i64>()?;

            constraint.check(
                signed_token_amount,
                spot_position.open_bids,
                spot_position.open_asks,
            )?;
        } else {
            let perp_position = match user.get_perp_position(constraint.market_index) {
                Ok(perp_position) => perp_position,
                Err(_) => continue,
            };

            constraint.check(
                perp_position.base_asset_amount,
                perp_position.open_bids,
                perp_position.open_asks,
            )?;
        }
    }

    Ok(())
}

#[derive(Accounts)]
pub struct CheckOrderConstraints<'info> {
    pub user: AccountLoader<'info, User>,
}

#[derive(Debug, Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Eq)]
pub struct OrderConstraint {
    pub max_position: i64,
    pub min_position: i64,
    pub market_index: u16,
    pub market_type: MarketType,
}

impl OrderConstraint {
    pub fn check(&self, current_position: i64, open_bids: i64, open_asks: i64) -> Result<()> {
        let max_long = current_position.safe_add(open_bids)?;

        if max_long > self.max_position {
            msg!(
                "market index {} market type {:?}",
                self.market_index,
                self.market_type
            );
            msg!(
                "max long {} current position {} open bids {}",
                max_long,
                current_position,
                open_bids
            );
            return Err(ErrorCode::OrderSizeBreached.into());
        }

        let max_short = current_position.safe_add(open_asks)?;
        if max_short < self.min_position {
            msg!(
                "market index {} market type {:?}",
                self.market_index,
                self.market_type
            );
            msg!(
                "max short {} current position {} open asks {}",
                max_short,
                current_position,
                open_asks
            );
            return Err(ErrorCode::OrderSizeBreached.into());
        }

        Ok(())
    }
}
