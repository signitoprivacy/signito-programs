use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    hash::hashv,
    program::{invoke, invoke_signed},
    system_instruction,
};

use crate::constants::TOKEN_2022_ID;
use crate::errors::SignitoError;
use crate::state::VaultState;

// Standard SPL Token account size: 165 bytes.
const SPL_TOKEN_ACCOUNT_LEN: usize = 165;
// Standard SPL Token Mint account size: 82 bytes.
const SPL_MINT_LEN: usize = 82;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ConvertToAirtokenArgs {
    // OTS preimage (same SHA-256 verification as unshield)
    pub ots_preimage: [u8; 32],
    // Amount in lamports to convert from sSOL to aSOL
    pub amount: u64,
}

#[derive(Accounts)]
pub struct ConvertToAirtoken<'info> {
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

    /// CHECK: Token-2022 sSOL mint, validated via address constraint
    #[account(mut, address = vault_pda.mint_stoken)]
    pub mint_stoken: UncheckedAccount<'info>,

    // Owner's sSOL token account (will be burned from)
    /// CHECK: sSOL token account for owner; caller must provide correct account
    #[account(mut)]
    pub owner_stoken_ata: UncheckedAccount<'info>,

    // New aToken (airSOL) mint: a fresh keypair supplied by the client.
    // Standard SPL Token (transferable, no NonTransferable extension).
    /// CHECK: created and initialised inside handler; must be a fresh keypair
    #[account(mut)]
    pub mint_atoken: Signer<'info>,

    // Escrow token account owned by vault_pda: holds aSOL until voucher is claimed.
    // Fresh keypair supplied by the client; created and initialised inside the handler.
    /// CHECK: created inside handler; must be a fresh keypair
    #[account(mut)]
    pub escrow_atoken_ata: Signer<'info>,

    /// CHECK: Token-2022 program (for sSOL burn)
    #[account(address = TOKEN_2022_ID)]
    pub token_program_2022: UncheckedAccount<'info>,

    /// CHECK: standard SPL Token program (for aToken mint)
    #[account(address = anchor_spl::token::ID)]
    pub token_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handler(ctx: Context<ConvertToAirtoken>, args: ConvertToAirtokenArgs) -> Result<()> {
    require!(args.amount > 0, SignitoError::InvalidAmount);

    let computed = hashv(&[args.ots_preimage.as_ref()]);

    let bump: u8;
    let vault_pda_key: Pubkey;
    let depth_remaining: u8;

    {
        let vault = &mut ctx.accounts.vault_pda;

        // OTS verification
        require!(
            computed.to_bytes() == vault.current_ots_hash,
            SignitoError::InvalidOtsPreimage
        );
        require!(vault.chain_depth > 0, SignitoError::VaultExhausted);
        require!(args.amount <= vault.total_deposited, SignitoError::InsufficientFunds);

        // Advance OTS chain
        vault.current_ots_hash = args.ots_preimage;
        vault.chain_depth = vault.chain_depth
            .checked_sub(1)
            .ok_or(SignitoError::Overflow)?;
        vault.total_deposited = vault.total_deposited
            .checked_sub(args.amount)
            .ok_or(SignitoError::Overflow)?;

        bump = vault.bump;
        depth_remaining = vault.chain_depth;
    }

    vault_pda_key = ctx.accounts.vault_pda.key();

    // Signer seeds for vault PDA CPIs (thaw, aToken mint, escrow).
    let seeds: &[&[&[u8]]] = &[&[b"vault", ctx.accounts.owner.key.as_ref(), &[bump]]];

    // The sSOL account is frozen while held by the user (freeze authority = vault_pda).
    // Conditionally thaw: new vaults have state=2 (frozen); old vaults may be state=1.
    // SPL Token classic account state byte is at offset 108.
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

    // Move SOL out of vault to owner (to fund aToken operations).
    {
        let vault_info = ctx.accounts.vault_pda.to_account_info();
        **vault_info.try_borrow_mut_lamports()? = vault_info
            .lamports()
            .checked_sub(args.amount)
            .ok_or(SignitoError::Overflow)?;
        **ctx.accounts.owner.to_account_info().try_borrow_mut_lamports()? = ctx
            .accounts
            .owner
            .lamports()
            .checked_add(args.amount)
            .ok_or(SignitoError::Overflow)?;
    }

    // ---- Create standard SPL Token mint for aToken (transferable) ----
    let mint_lamports = ctx.accounts.rent.minimum_balance(SPL_MINT_LEN);

    invoke(
        &system_instruction::create_account(
            ctx.accounts.owner.key,
            ctx.accounts.mint_atoken.key,
            mint_lamports,
            SPL_MINT_LEN as u64,
            &anchor_spl::token::ID,
        ),
        &[
            ctx.accounts.owner.to_account_info(),
            ctx.accounts.mint_atoken.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // Vault PDA is mint authority: only this program can mint aTokens.
    invoke_signed(
        &spl_token::instruction::initialize_mint(
            &anchor_spl::token::ID,
            ctx.accounts.mint_atoken.key,
            &vault_pda_key,
            None,
            9,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.mint_atoken.to_account_info(),
            ctx.accounts.rent.to_account_info(),
        ],
        seeds,
    )?;

    // ---- Create escrow token account owned by vault_pda directly (no ATA program) ----
    let escrow_lamports = ctx.accounts.rent.minimum_balance(SPL_TOKEN_ACCOUNT_LEN);

    invoke(
        &system_instruction::create_account(
            ctx.accounts.owner.key,
            ctx.accounts.escrow_atoken_ata.key,
            escrow_lamports,
            SPL_TOKEN_ACCOUNT_LEN as u64,
            &anchor_spl::token::ID,
        ),
        &[
            ctx.accounts.owner.to_account_info(),
            ctx.accounts.escrow_atoken_ata.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // Initialize escrow token account (owned by vault_pda as authority).
    // initialize_account3 does not need the rent sysvar.
    invoke_signed(
        &spl_token::instruction::initialize_account3(
            &anchor_spl::token::ID,
            ctx.accounts.escrow_atoken_ata.key,
            ctx.accounts.mint_atoken.key,
            &vault_pda_key,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.escrow_atoken_ata.to_account_info(),
            ctx.accounts.mint_atoken.to_account_info(),
        ],
        seeds,
    )?;

    // Mint aSOL to escrow token account (vault PDA signs as mint authority).
    invoke_signed(
        &spl_token::instruction::mint_to(
            &anchor_spl::token::ID,
            ctx.accounts.mint_atoken.key,
            ctx.accounts.escrow_atoken_ata.key,
            &vault_pda_key,
            &[],
            args.amount,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.mint_atoken.to_account_info(),
            ctx.accounts.escrow_atoken_ata.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
        ],
        seeds,
    )?;

    msg!(
        "Converted {} lamports: sSOL burned, aSOL in escrow. aToken: {}. OTS depth: {}",
        args.amount,
        ctx.accounts.mint_atoken.key,
        depth_remaining,
    );

    Ok(())
}
