use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hashv;
use anchor_lang::solana_program::program::{invoke, invoke_signed};

use crate::constants::TOKEN_2022_ID;
use crate::errors::SignitoError;
use crate::state::VaultState;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct UnshieldArgs {
    // H_{n-1}: revealed preimage of the current chain tip.
    // Program verifies: SHA-256(ots_preimage) == vault.current_ots_hash
    pub ots_preimage: [u8; 32],
    // Lamports to withdraw. Must be <= vault.total_deposited.
    pub amount: u64,
}

#[derive(Accounts)]
pub struct Unshield<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", owner.key().as_ref()],
        bump = vault_pda.bump,
        has_one = owner,
        has_one = mint_stoken,
    )]
    pub vault_pda: Account<'info, VaultState>,

    /// CHECK: Token-2022 mint for this vault, validated via has_one + address constraint
    #[account(mut, address = vault_pda.mint_stoken)]
    pub mint_stoken: UncheckedAccount<'info>,

    // Owner's sSOL token account (Token-2022)
    /// CHECK: ATA for owner's sSOL, caller must provide correct account
    #[account(mut)]
    pub owner_stoken_ata: UncheckedAccount<'info>,

    // SOL destination (can be a fresh address with no prior on-chain history)
    /// CHECK: any valid account, can be fresh
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,

    /// CHECK: Token-2022 program
    #[account(address = TOKEN_2022_ID)]
    pub token_program_2022: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<Unshield>, args: UnshieldArgs) -> Result<()> {
    require!(args.amount > 0, SignitoError::InvalidAmount);

    // FIX: compute hash before taking the mutable borrow, using .as_ref() for coercion.
    // hashv expects &[&[u8]]; .as_ref() on [u8; 32] produces &[u8].
    let computed = hashv(&[args.ots_preimage.as_ref()]);

    // Track chain_depth for the msg! after we drop the mutable borrow.
    let depth_remaining: u8;

    // FIX: scope the mutable borrow so it is dropped before the CPI calls below,
    // which also need to access ctx.accounts.vault_pda via to_account_info().
    {
        let vault = &mut ctx.accounts.vault_pda;

        // OTS verification: SHA-256(preimage) must equal stored chain tip
        require!(
            computed.to_bytes() == vault.current_ots_hash,
            SignitoError::InvalidOtsPreimage
        );
        require!(vault.chain_depth > 0, SignitoError::VaultExhausted);
        require!(args.amount <= vault.total_deposited, SignitoError::InsufficientFunds);

        // Advance the chain: tip becomes the revealed preimage (one step closer to H0)
        vault.current_ots_hash = args.ots_preimage;
        vault.chain_depth = vault.chain_depth
            .checked_sub(1)
            .ok_or(SignitoError::Overflow)?;
        vault.total_deposited = vault.total_deposited
            .checked_sub(args.amount)
            .ok_or(SignitoError::Overflow)?;

        depth_remaining = vault.chain_depth;
    }
    // vault mutable borrow is dropped here

    // The sSOL receipt account is frozen at all times while held by the user.
    // Only the Signito program (vault_pda, as freeze authority) can thaw it.
    // Step A: thaw the account so the burn instruction can execute.
    // Step B: burn the sSOL — vault_pda signs as mint authority / token authority.
    // Both steps use invoke_signed with vault_pda seeds so no external party
    // can replicate this sequence without a valid OTS proof (verified above).
    let vault_pda_key = ctx.accounts.vault_pda.key();
    let bump = ctx.accounts.vault_pda.bump;
    let seeds: &[&[&[u8]]] = &[&[b"vault", ctx.accounts.owner.key.as_ref(), &[bump]]];

    // Step A: conditionally thaw.
    // New vaults created by this program have their sSOL account frozen (state=2).
    // Vaults from older program versions may have state=1 (initialized, not frozen).
    // SPL Token classic account state byte is at offset 108:
    //   mint(32) + owner(32) + amount(8) + delegate_option(4) + delegate(32) = 108
    //   then state(1): 0=Uninit, 1=Init, 2=Frozen
    let is_frozen = ctx
        .accounts
        .owner_stoken_ata
        .data
        .borrow()
        .get(108)
        .copied()
        == Some(2);

    if is_frozen {
        invoke_signed(
            &spl_token_2022::instruction::thaw_account(
                &TOKEN_2022_ID,
                ctx.accounts.owner_stoken_ata.key,
                ctx.accounts.mint_stoken.key,
                &vault_pda_key,
                &[],
            )
            .map_err(|_| error!(SignitoError::Overflow))?,
            &[
                ctx.accounts.owner_stoken_ata.to_account_info(),
                ctx.accounts.mint_stoken.to_account_info(),
                ctx.accounts.vault_pda.to_account_info(),
            ],
            seeds,
        )?;
    }

    // Step B: burn — authority is the owner's wallet (already a Signer in this tx).
    // Owner's signature propagates into CPI via invoke (no PDA seeds needed here).
    invoke(
        &spl_token_2022::instruction::burn(
            &TOKEN_2022_ID,
            ctx.accounts.owner_stoken_ata.key,
            ctx.accounts.mint_stoken.key,
            ctx.accounts.owner.key,
            &[],
            args.amount,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.mint_stoken.to_account_info(),
            ctx.accounts.owner.to_account_info(),
        ],
    )?;

    // Step C: re-freeze the sSOL account so the remaining balance stays shielded.
    // After a partial unshield the account still holds sSOL; we must re-freeze it
    // so the holder cannot freely burn or transfer the remaining receipt tokens.
    // Only the vault PDA (freeze authority) can thaw it again on the next unshield.
    invoke_signed(
        &spl_token_2022::instruction::freeze_account(
            &TOKEN_2022_ID,
            ctx.accounts.owner_stoken_ata.key,
            ctx.accounts.mint_stoken.key,
            &vault_pda_key,
            &[],
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.mint_stoken.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
        ],
        seeds,
    )?;

    // Transfer SOL from vault PDA to destination.
    // vault_pda is program-owned: use direct lamport manipulation (system_program::transfer
    // cannot move SOL out of a program-owned account).
    // We only move the deposit portion; the rent minimum stays in vault_pda.
    {
        // FIX: to_account_info() is safe here because the vault mutable borrow is dropped.
        let vault_info = ctx.accounts.vault_pda.to_account_info();
        let dest_info = ctx.accounts.destination.to_account_info();
        **vault_info.try_borrow_mut_lamports()? = vault_info
            .lamports()
            .checked_sub(args.amount)
            .ok_or(SignitoError::Overflow)?;
        **dest_info.try_borrow_mut_lamports()? = dest_info
            .lamports()
            .checked_add(args.amount)
            .ok_or(SignitoError::Overflow)?;
    }

    msg!(
        "Unshield: {} lamports -> {}. OTS depth remaining: {}",
        args.amount,
        ctx.accounts.destination.key,
        depth_remaining,
    );

    Ok(())
}
