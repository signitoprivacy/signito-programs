use anchor_lang::prelude::*;

use crate::errors::SignitoError;
use crate::state::VaultState;

#[derive(Accounts)]
pub struct CloseVault<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    // `close = owner` sends all remaining lamports (rent) back to owner.
    // Constraint: vault must be empty before closing.
    #[account(
        mut,
        seeds = [b"vault", owner.key().as_ref()],
        bump = vault_pda.bump,
        has_one = owner,
        constraint = vault_pda.total_deposited == 0 @ SignitoError::VaultNotEmpty,
        close = owner,
    )]
    pub vault_pda: Account<'info, VaultState>,

    pub system_program: Program<'info, System>,
}

pub fn handler(_ctx: Context<CloseVault>) -> Result<()> {
    // Anchor's `close = owner` constraint handles the rent transfer automatically.
    // All validation is done via account constraints.
    msg!("Vault closed. Account rent returned to owner.");
    Ok(())
}
