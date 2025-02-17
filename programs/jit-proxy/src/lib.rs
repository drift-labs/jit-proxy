use anchor_lang::prelude::*;

pub mod error;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP");

#[program]
pub mod jit_proxy {
    use super::*;

    pub fn jit<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, Jit<'info>>,
        params: JitParams,
    ) -> Result<()> {
        instructions::jit(ctx, params)
    }

    pub fn jit_signed_msg<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, JitSignedMsg<'info>>,
        params: JitSignedMsgParams,
    ) -> Result<()> {
        instructions::jit_signed_msg(ctx, params)
    }

    pub fn check_order_constraints<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CheckOrderConstraints<'info>>,
        constraints: Vec<OrderConstraint>,
    ) -> Result<()> {
        instructions::check_order_constraints(ctx, constraints)
    }

    pub fn arb_perp<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, ArbPerp<'info>>,
        market_index: u16,
    ) -> Result<()> {
        instructions::arb_perp(ctx, market_index)
    }
}
