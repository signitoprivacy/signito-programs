use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hashv;
use anchor_lang::solana_program::program::invoke_signed;

use crate::constants::TOKEN_2022_ID;
use crate::errors::SignitoError;
use crate::state::VaultState;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ZkUnshieldArgs {
    // H_{n-1}: revealed preimage of the current chain tip.
    // Program verifies: SHA-256(ots_preimage) == vault.current_ots_hash
    pub ots_preimage: [u8; 32],
    // Lamports to withdraw. Must be <= vault.total_deposited.
    pub amount: u64,
}

// ZkUnshield: relayer-mediated unshield with no owner signature on-chain.
//
// The vault owner proves control via OTS preimage (off-chain knowledge).
// The relayer signs to pay transaction fees and broadcast the transaction.
// The owner's wallet address never appears as a signer, preserving privacy.
//
// Requires: initialize_vault (or deposit) must have called approve(vault_pda, u64::MAX)
// to grant the vault PDA delegate authority over the sSOL token account.
#[derive(Accounts)]
pub struct ZkUnshield<'info> {
    // Relayer pays the transaction fee. Does NOT need to be the vault owner.
    #[account(mut)]
    pub relayer: Signer<'info>,

    // Vault owner pubkey (readonly, not signer).
    // Used only for PDA derivation and has_one validation.
    // NOT present as a transaction signer — this is the privacy guarantee.
    /// CHECK: used only for PDA derivation; not a signer
    pub owner: UncheckedAccount<'info>,

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

    // Owner's sSOL token account (Token-2022).
    // Must have vault_pda set as delegate via approve (done in initialize_vault / deposit).
    /// CHECK: sSOL token account; delegate must be vault_pda (checked in handler)
    #[account(mut)]
    pub owner_stoken_ata: UncheckedAccount<'info>,

    // SOL destination (can be a fresh address with no prior on-chain history).
    /// CHECK: any valid account, can be fresh
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,

    /// CHECK: Token-2022 program
    #[account(address = TOKEN_2022_ID)]
    pub token_program_2022: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ZkUnshield>, args: ZkUnshieldArgs) -> Result<()> {
    require!(args.amount > 0, SignitoError::InvalidAmount);

    // Compute SHA-256(ots_preimage) before taking mutable borrow of vault_pda.
    let computed = hashv(&[args.ots_preimage.as_ref()]);

    let depth_remaining: u8;

    {
        let vault = &mut ctx.accounts.vault_pda;

        // OTS verification: SHA-256(preimage) must equal stored chain tip.
        require!(
            computed.to_bytes() == vault.current_ots_hash,
            SignitoError::InvalidOtsPreimage
        );
        require!(vault.chain_depth > 0, SignitoError::VaultExhausted);
        require!(args.amount <= vault.total_deposited, SignitoError::InsufficientFunds);

        // Verify vault_pda is set as the delegate on the sSOL token account.
        //
        // SPL Token classic account layout (165 bytes):
        //   [0..32]   mint
        //   [32..64]  owner
        //   [64..72]  amount (u64 LE)
        //   [72..76]  delegate_option (u32 LE): 0 = none, 1 = some
        //   [76..108] delegate (Pubkey, 32 bytes)
        //   [108]     state (1=initialized, 2=frozen)
        //   ... rest
        let data = ctx.accounts.owner_stoken_ata.data.borrow();

        require!(data.len() >= 108, SignitoError::Unauthorized);

        let delegate_option = u32::from_le_bytes(
            data[72..76]
                .try_into()
                .map_err(|_| error!(SignitoError::Unauthorized))?,
        );

        require!(delegate_option == 1, SignitoError::Unauthorized);

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&data[76..108]);
        let delegate_key = Pubkey::from(key_bytes);

        require!(delegate_key == vault.key(), SignitoError::Unauthorized);

        drop(data);

        // Advance the OTS chain: tip becomes the revealed preimage (one step closer to H0).
        vault.current_ots_hash = args.ots_preimage;
        vault.chain_depth = vault
            .chain_depth
            .checked_sub(1)
            .ok_or(SignitoError::Overflow)?;
        vault.total_deposited = vault
            .total_deposited
            .checked_sub(args.amount)
            .ok_or(SignitoError::Overflow)?;

        depth_remaining = vault.chain_depth;
    }
    // vault mutable borrow dropped here

    let vault_pda_key = ctx.accounts.vault_pda.key();
    let bump = ctx.accounts.vault_pda.bump;
    let seeds: &[&[&[u8]]] = &[&[b"vault", ctx.accounts.owner.key.as_ref(), &[bump]]];

    // Step A: conditionally thaw the sSOL account.
    // The account is normally frozen between operations.
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

    // Step B: burn sSOL via vault PDA as delegate authority.
    // The vault PDA was approved as delegate with u64::MAX in initialize_vault / deposit.
    // invoke_signed allows the vault PDA to authorize the burn without the owner signing.
    invoke_signed(
        &spl_token_2022::instruction::burn(
            &TOKEN_2022_ID,
            ctx.accounts.owner_stoken_ata.key,
            ctx.accounts.mint_stoken.key,
            &vault_pda_key,
            &[],
            args.amount,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.owner_stoken_ata.to_account_info(),
            ctx.accounts.mint_stoken.to_account_info(),
            ctx.accounts.vault_pda.to_account_info(),
        ],
        seeds,
    )?;

    // Step C: re-freeze the sSOL account so remaining balance stays shielded.
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

    // Step D: transfer SOL from vault PDA to destination.
    // vault_pda is program-owned so we use direct lamport manipulation.
    // Only the deposit portion is moved; rent minimum stays in vault_pda.
    {
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
        "ZkUnshield: {} lamports -> {}. OTS depth remaining: {}",
        args.amount,
        ctx.accounts.destination.key,
        depth_remaining,
    );

    Ok(())
}
