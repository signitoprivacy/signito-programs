use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};

use crate::constants::TOKEN_2022_ID;
use crate::errors::SignitoError;
use crate::state::VaultState;

// Token-2022 mint: exactly Mint::LEN = 82 bytes.
// Using 82 bytes means `rest.is_empty() = true` inside Token-2022's unpack_uninitialized,
// which skips extension TLV parsing entirely and avoids the InvalidAccountData that
// triggers when rest.len() (1 for 83-byte) <= account_type_index (84).
// 83 bytes causes the extension-parsing path to run and fail on devnet's deployed Token-2022.
const MINT_LEN: usize = 82;

// Classic 165-byte token account (Token-2022 accepts this without extensions).
const TOKEN_ACCOUNT_LEN: usize = 165;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitializeVaultArgs {
    pub ots_tip: [u8; 32],
    pub chain_depth: u8,
    pub amount: u64,
}

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = VaultState::LEN,
        seeds = [b"vault", owner.key().as_ref()],
        bump,
    )]
    pub vault_pda: Account<'info, VaultState>,

    /// CHECK: created and initialised via CPI inside handler; must be a fresh keypair
    #[account(mut)]
    pub mint_stoken: Signer<'info>,

    /// CHECK: created and initialised via CPI inside handler; must be a fresh keypair
    #[account(mut)]
    pub owner_stoken_ata: Signer<'info>,

    pub system_program: Program<'info, System>,

    /// CHECK: Token-2022 program
    #[account(address = TOKEN_2022_ID)]
    pub token_program_2022: UncheckedAccount<'info>,
}

pub fn handler(ctx: Context<InitializeVault>, args: InitializeVaultArgs) -> Result<()> {
    require!(args.amount > 0, SignitoError::InvalidAmount);
    require!(
        args.chain_depth > 0 && args.chain_depth <= 64,
        SignitoError::InvalidAmount
    );

    let bump = ctx.bumps.vault_pda;
    let vault_pda_key = ctx.accounts.vault_pda.key();
    let seeds: &[&[&[u8]]] = &[&[b"vault", ctx.accounts.owner.key.as_ref(), &[bump]]];

    // Persist vault state.
    {
        let vault = &mut ctx.accounts.vault_pda;
        vault.owner = ctx.accounts.owner.key();
        vault.current_ots_hash = args.ots_tip;
        vault.chain_depth = args.chain_depth;
        vault.mint_stoken = ctx.accounts.mint_stoken.key();
        vault.total_deposited = args.amount;
        vault.bump = bump;
    }

    let rent = Rent::get()?;

    // ---- 1. Create Token-2022 mint account ----
    let mint_lamports = rent.minimum_balance(MINT_LEN);

    invoke(
        &system_instruction::create_account(
            ctx.accounts.owner.key,
            ctx.accounts.mint_stoken.key,
            mint_lamports,
            MINT_LEN as u64,
            &TOKEN_2022_ID,
        ),
        &[
            ctx.accounts.owner.to_account_info(),
            ctx.accounts.mint_stoken.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // ---- 2. Initialize the mint ----
    // mint_authority = vault PDA (only this program can mint sSOL).
    // freeze_authority = vault PDA (program can freeze/thaw the receipt account).
    invoke(
        &spl_token_2022::instruction::initialize_mint2(
            &TOKEN_2022_ID,
            ctx.accounts.mint_stoken.key,
            &vault_pda_key,
            Some(&vault_pda_key),
            9,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[ctx.accounts.mint_stoken.to_account_info()],
    )?;

    // ---- 3. Create owner sToken account (classic 165-byte Token-2022 path) ----
    let account_lamports = rent.minimum_balance(TOKEN_ACCOUNT_LEN);

    invoke(
        &system_instruction::create_account(
            ctx.accounts.owner.key,
            ctx.accounts.owner_stoken_ata.key,
            account_lamports,
            TOKEN_ACCOUNT_LEN as u64,
            &TOKEN_2022_ID,
        ),
        &[
            ctx.accounts.owner.to_account_info(),
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;

    // ---- 4. Initialize the sToken account ----
    invoke(
        &spl_token_2022::instruction::initialize_account3(
            &TOKEN_2022_ID,
            ctx.accounts.owner_stoken_ata.key,
            ctx.accounts.mint_stoken.key,
            ctx.accounts.owner.key,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.mint_stoken.to_account_info(),
        ],
    )?;

    // ---- 5. Mint sSOL to owner sToken account (vault PDA signs as mint authority) ----
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

    // ---- 5b. Approve vault PDA as delegate on the sSOL account ----
    // This grants the vault PDA authority to burn sSOL on behalf of the owner
    // without requiring the owner's wallet signature (used by zk_unshield).
    invoke(
        &spl_token_2022::instruction::approve(
            &TOKEN_2022_ID,
            ctx.accounts.owner_stoken_ata.key,
            &vault_pda_key,
            ctx.accounts.owner.key,
            &[],
            u64::MAX,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
            ctx.accounts.owner.to_account_info(),
        ],
    )?;

    // ---- 6. Freeze the sSOL token account (vault PDA signs as freeze authority) ----
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

    // ---- 7. Transfer SOL deposit from owner to vault_pda ----
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

    msg!(
        "Vault initialised. OTS depth: {}. Deposited: {} lamports. Mint: {} (sSOL, Token-2022)",
        args.chain_depth,
        args.amount,
        ctx.accounts.mint_stoken.key,
    );

    Ok(())
}
