use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;

use crate::constants::TOKEN_2022_ID;
use crate::errors::SignitoError;
use crate::state::VaultState;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct DepositArgs {
    pub amount: u64,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
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

    /// CHECK: Token-2022 mint for this vault, validated via has_one
    #[account(mut, address = vault_pda.mint_stoken)]
    pub mint_stoken: UncheckedAccount<'info>,

    /// CHECK: Owner's sSOL token account (Token-2022, created at vault init)
    #[account(mut)]
    pub owner_stoken_ata: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,

    /// CHECK: Token-2022 program
    #[account(address = TOKEN_2022_ID)]
    pub token_program_2022: UncheckedAccount<'info>,
}

pub fn handler(ctx: Context<Deposit>, args: DepositArgs) -> Result<()> {
    require!(args.amount > 0, SignitoError::InvalidAmount);

    let bump = ctx.accounts.vault_pda.bump;
    let vault_pda_key = ctx.accounts.vault_pda.key();
    let seeds: &[&[&[u8]]] = &[&[b"vault", ctx.accounts.owner.key.as_ref(), &[bump]]];

    {
        let vault = &mut ctx.accounts.vault_pda;
        vault.total_deposited = vault
            .total_deposited
            .checked_add(args.amount)
            .ok_or(SignitoError::Overflow)?;
    }

    // Transfer SOL from owner to vault_pda.
    // vault_pda is program-owned so owner (signer) transfers into it via System Program.
    invoke(
        &system_instruction::transfer(
            ctx.accounts.owner.key,
            &vault_pda_key,
            args.amount,
        ),
        &[
            ctx.accounts.owner.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // Thaw sSOL account so mint_to can execute (vault_pda is freeze authority).
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

    // Mint additional sSOL to owner's token account (vault_pda is mint authority).
    invoke_signed(
        &spl_token_2022::instruction::mint_to(
            &TOKEN_2022_ID,
            ctx.accounts.mint_stoken.key,
            ctx.accounts.owner_stoken_ata.key,
            &vault_pda_key,
            &[],
            args.amount,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.mint_stoken.to_account_info(),
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
        ],
        seeds,
    )?;

    // Re-approve vault PDA as delegate after minting more sSOL.
    // This refreshes / sets the delegation for vaults created before the zk_unshield upgrade.
    invoke_signed(
        &spl_token_2022::instruction::approve(
            &TOKEN_2022_ID,
            ctx.accounts.owner_stoken_ata.key,
            &vault_pda_key,
            &vault_pda_key,
            &[],
            u64::MAX,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
        ],
        seeds,
    )?;

    // Re-freeze the sSOL account to keep the receipt non-transferable.
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

    msg!(
        "Deposit: {} lamports added to vault. Total: {}",
        args.amount,
        ctx.accounts.vault_pda.total_deposited,
    );

    Ok(())
}
