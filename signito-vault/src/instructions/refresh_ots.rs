use anchor_lang::prelude::*;
use crate::state::VaultState;

// RefreshOts: resets the OTS chain on an existing vault without closing it.
//
// When a chain is exhausted (chain_depth == 0), or approaching it, the owner
// may supply a new OTS chain derived from the same vault code with the next
// generation suffix:
//   gen 0: PBKDF2(vaultCode, walletAddress)
//   gen N: PBKDF2(vaultCode, walletAddress + ":gen:" + N)
//
// Security: only the vault owner (who must sign this transaction) can reset
// the chain. The new tip is chosen by the owner, so they control the chain.
// This is correct because the OTS chain protects against unauthorized
// withdrawals, not against the vault owner themselves.

#[derive(Accounts)]
pub struct RefreshOts<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", owner.key().as_ref()],
        bump = vault_pda.bump,
        has_one = owner,
    )]
    pub vault_pda: Account<'info, VaultState>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct RefreshOtsArgs {
    pub new_ots_tip: [u8; 32],
    pub new_chain_depth: u8,
}

pub fn handler(ctx: Context<RefreshOts>, args: RefreshOtsArgs) -> Result<()> {
    let vault = &mut ctx.accounts.vault_pda;
    vault.current_ots_hash = args.new_ots_tip;
    vault.chain_depth = args.new_chain_depth;
    Ok(())
}
