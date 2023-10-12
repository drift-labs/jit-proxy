use anchor_lang::prelude::*;

pub mod error;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP");

#[program]
pub mod jit_proxy {
    use super::*;

    pub fn jit<'info>(
        ctx: Context<'_, '_, '_, 'info, Jit<'info>>,
        params: JitParams,
    ) -> Result<()> {
        instructions::jit(ctx, params)
    }

    pub fn check_order_constraints<'info>(
        ctx: Context<'_, '_, '_, 'info, CheckOrderConstraints<'info>>,
        constraints: Vec<OrderConstraint>,
    ) -> Result<()> {
        instructions::check_order_constraints(ctx, constraints)
    }
}
