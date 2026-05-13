use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;

use instructions::claim_voucher::*;
use instructions::close_vault::*;
use instructions::convert_to_airtoken::*;
use instructions::deposit::*;
use instructions::initialize_vault::*;
use instructions::refresh_ots::*;
use instructions::unshield::*;
use instructions::zk_unshield::*;

declare_id!("9PibgJMUa3zXVd7YWJEJ8UQ14A7z2J3qZ7QDvRW38XeD");

#[program]
pub mod signito_vault {
    use super::*;

    // Deposit SOL, set OTS root hash, mint sSOL (NonTransferable Token-2022) to owner.
    pub fn initialize_vault(
        ctx: Context<InitializeVault>,
        args: InitializeVaultArgs,
    ) -> Result<()> {
        instructions::initialize_vault::handler(ctx, args)
    }

    // Verify OTS preimage on-chain (SHA-256), burn sSOL, transfer SOL to destination.
    pub fn unshield(ctx: Context<Unshield>, args: UnshieldArgs) -> Result<()> {
        instructions::unshield::handler(ctx, args)
    }

    // Deposit additional SOL into an existing vault and mint sSOL receipt tokens.
    pub fn deposit(ctx: Context<Deposit>, args: DepositArgs) -> Result<()> {
        instructions::deposit::handler(ctx, args)
    }

    // Close an empty vault, return account rent to owner.
    pub fn close_vault(ctx: Context<CloseVault>) -> Result<()> {
        instructions::close_vault::handler(ctx)
    }

    // Burn sSOL (OTS-verified), mint aSOL (transferable) for offline voucher use.
    pub fn convert_to_airtoken(
        ctx: Context<ConvertToAirtoken>,
        args: ConvertToAirtokenArgs,
    ) -> Result<()> {
        instructions::convert_to_airtoken::handler(ctx, args)
    }

    // Verify Ed25519 voucher signature (via instructions sysvar), transfer aSOL to claimer.
    pub fn claim_voucher(
        ctx: Context<ClaimVoucher>,
        args: ClaimVoucherArgs,
    ) -> Result<()> {
        instructions::claim_voucher::handler(ctx, args)
    }

    // Reset the OTS chain on an existing vault without closing it.
    // Caller must supply a new chain tip derived from the same vault code + next generation.
    // Only the vault owner (signer) may call this.
    pub fn refresh_ots(
        ctx: Context<RefreshOts>,
        args: RefreshOtsArgs,
    ) -> Result<()> {
        instructions::refresh_ots::handler(ctx, args)
    }

    // Relayer-mediated unshield: OTS verified on-chain, sSOL burned via vault PDA delegate,
    // SOL sent from vault PDA to destination. Owner wallet does NOT sign.
    // Requires initialize_vault or deposit to have called approve(vault_pda, u64::MAX).
    pub fn zk_unshield(
        ctx: Context<ZkUnshield>,
        args: ZkUnshieldArgs,
    ) -> Result<()> {
        instructions::zk_unshield::handler(ctx, args)
    }
}
