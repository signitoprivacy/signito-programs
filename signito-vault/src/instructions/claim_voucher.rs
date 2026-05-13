use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    ed25519_program,
    program::invoke,
    program::invoke_signed,
    sysvar::instructions::{load_instruction_at_checked, ID as INSTRUCTIONS_ID},
};

use crate::errors::SignitoError;
use crate::state::NonceRecord;

// Binary voucher message format (64 bytes total, signed by issuer wallet):
//
//   [0..8]   amount: u64 LE (lamports)
//   [8..40]  recipient: Pubkey (32 bytes)
//   [40..56] nonce: [u8; 16]  -- also used as the NonceRecord PDA seed
//   [56..64] expires_at: i64 LE (Unix timestamp, seconds)
//
// The client calls wallet.signMessage(voucher_msg) and the resulting 64-byte
// Ed25519 signature is passed as `args.sig`.
pub const VOUCHER_MSG_LEN: usize = 64;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ClaimVoucherArgs {
    // The 64-byte binary voucher message that was signed by the issuer
    pub voucher_msg: [u8; VOUCHER_MSG_LEN],
    // Ed25519 signature (64 bytes) over voucher_msg by the issuer
    pub sig: [u8; 64],
}

#[derive(Accounts)]
#[instruction(args: ClaimVoucherArgs)]
pub struct ClaimVoucher<'info> {
    // The recipient who is claiming the voucher (pays for the nonce PDA)
    #[account(mut)]
    pub claimer: Signer<'info>,

    // The wallet that issued the voucher; verified against the Ed25519 instruction
    /// CHECK: pubkey verified via Ed25519SigVerify sysvar check inside handler
    pub issuer: UncheckedAccount<'info>,

    // Issuer's vault PDA: signing authority for the escrow ATA.
    // Seeds: [b"vault", issuer.key()]
    /// CHECK: PDA derived from issuer; verified by seeds constraint
    #[account(
        seeds = [b"vault", issuer.key().as_ref()],
        bump,
    )]
    pub issuer_vault_pda: UncheckedAccount<'info>,

    // aToken mint created by convert_to_airtoken
    /// CHECK: standard SPL Token mint
    #[account(mut)]
    pub mint_atoken: UncheckedAccount<'info>,

    // Escrow ATA owned by issuer_vault_pda (source of funds)
    /// CHECK: ATA owned by issuer_vault_pda for mint_atoken
    #[account(mut)]
    pub escrow_atoken_ata: UncheckedAccount<'info>,

    // Claimer's aToken ATA (destination, created if missing)
    /// CHECK: ATA for claimer
    #[account(mut)]
    pub claimer_atoken_ata: UncheckedAccount<'info>,

    // Nonce record PDA: `init` fails if it already exists, preventing replay.
    // Seeds: [b"nonce", mint_atoken.key, nonce_bytes(16 from voucher_msg[40..56])]
    #[account(
        init,
        payer = claimer,
        space = NonceRecord::LEN,
        seeds = [
            b"nonce",
            mint_atoken.key().as_ref(),
            &args.voucher_msg[40..56],
        ],
        bump,
    )]
    pub nonce_pda: Account<'info, NonceRecord>,

    // Instructions sysvar -- needed to read the Ed25519SigVerify instruction
    /// CHECK: validated by address constraint
    #[account(address = INSTRUCTIONS_ID)]
    pub instructions: UncheckedAccount<'info>,

    /// CHECK: standard SPL Token program
    pub token_program: UncheckedAccount<'info>,

    /// CHECK: Associated Token program
    pub associated_token_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ClaimVoucher>, args: ClaimVoucherArgs) -> Result<()> {
    // Parse binary voucher message fields
    let amount = u64::from_le_bytes(
        args.voucher_msg[0..8].try_into().unwrap(),
    );

    let recipient_bytes: [u8; 32] = args.voucher_msg[8..40]
        .try_into()
        .map_err(|_| error!(SignitoError::InvalidVoucherSig))?;
    let recipient = Pubkey::from(recipient_bytes);

    let expires_at = i64::from_le_bytes(
        args.voucher_msg[56..64].try_into().unwrap(),
    );

    require!(amount > 0, SignitoError::InvalidAmount);

    // Verify recipient matches the claimer
    require!(
        recipient == ctx.accounts.claimer.key(),
        SignitoError::RecipientMismatch
    );

    // Verify expiry
    let clock = Clock::get()?;
    require!(clock.unix_timestamp < expires_at, SignitoError::VoucherExpired);

    // Verify Ed25519 signature via instructions sysvar.
    // The transaction MUST include an Ed25519SigVerify instruction at index 0.
    let ed25519_ix = load_instruction_at_checked(
        0,
        &ctx.accounts.instructions.to_account_info(),
    )
    .map_err(|_| error!(SignitoError::InvalidVoucherSig))?;

    verify_ed25519_ix(
        &ed25519_ix,
        ctx.accounts.issuer.key.as_ref(),
        &args.voucher_msg,
        &args.sig,
    )?;

    // Mark nonce as claimed (init already ensures uniqueness).
    ctx.accounts.nonce_pda.claimed_at = clock.unix_timestamp;

    // Create claimer's token account directly if it does not exist yet.
    // Uses System Program + SPL Token directly (no ATA program dependency).
    if ctx.accounts.claimer_atoken_ata.data_is_empty() {
        const SPL_TOKEN_ACCOUNT_LEN: usize = 165;
        let rent = Rent::get()?;
        let account_lamports = rent.minimum_balance(SPL_TOKEN_ACCOUNT_LEN);
        invoke(
            &anchor_lang::solana_program::system_instruction::create_account(
                ctx.accounts.claimer.key,
                ctx.accounts.claimer_atoken_ata.key,
                account_lamports,
                SPL_TOKEN_ACCOUNT_LEN as u64,
                &spl_token::ID,
            ),
            &[
                ctx.accounts.claimer.to_account_info(),
                ctx.accounts.claimer_atoken_ata.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        invoke(
            &spl_token::instruction::initialize_account3(
                &spl_token::ID,
                ctx.accounts.claimer_atoken_ata.key,
                ctx.accounts.mint_atoken.key,
                ctx.accounts.claimer.key,
            )
            .map_err(|_| error!(SignitoError::Overflow))?,
            &[
                ctx.accounts.claimer_atoken_ata.to_account_info(),
                ctx.accounts.mint_atoken.to_account_info(),
            ],
        )?;
    }

    // Transfer aSOL from escrow (owned by issuer_vault_pda) to claimer.
    // issuer_vault_pda signs via seeds.
    let issuer_key = ctx.accounts.issuer.key();
    let vault_bump = ctx.bumps.issuer_vault_pda;
    let vault_seeds: &[&[&[u8]]] = &[&[b"vault", issuer_key.as_ref(), &[vault_bump]]];

    invoke_signed(
        &spl_token::instruction::transfer(
            &spl_token::ID,
            ctx.accounts.escrow_atoken_ata.key,
            ctx.accounts.claimer_atoken_ata.key,
            &ctx.accounts.issuer_vault_pda.key(),
            &[],
            amount,
        )
        .map_err(|_| error!(SignitoError::Overflow))?,
        &[
            ctx.accounts.escrow_atoken_ata.to_account_info(),
            ctx.accounts.claimer_atoken_ata.to_account_info(),
            ctx.accounts.issuer_vault_pda.to_account_info(),
        ],
        vault_seeds,
    )?;

    emit!(VoucherClaimed {
        claimer: ctx.accounts.claimer.key(),
        issuer: ctx.accounts.issuer.key(),
        mint_atoken: ctx.accounts.mint_atoken.key(),
        amount,
        claimed_at: clock.unix_timestamp,
    });

    msg!(
        "Voucher claimed: {} lamports aSOL from escrow to {}",
        amount,
        ctx.accounts.claimer.key,
    );

    Ok(())
}

// Verify that instruction at index 0 is a valid Ed25519SigVerify matching our sig/pubkey/msg.
//
// Ed25519SigVerify instruction data layout:
//   [0]      num_signatures (u8) -- must be 1
//   [1]      padding (u8)        -- must be 0
//   Per signature, 14-byte header at offset 2:
//     [0..2]   sig_offset (u16 LE)
//     [2..4]   sig_ix_index (u16 LE)
//     [4..6]   pubkey_offset (u16 LE)
//     [6..8]   pubkey_ix_index (u16 LE)
//     [8..10]  msg_offset (u16 LE)
//     [10..12] msg_size (u16 LE)
//     [12..14] msg_ix_index (u16 LE)
//   Followed by: [sig (64)] [pubkey (32)] [msg (N)]
fn verify_ed25519_ix(
    ix: &anchor_lang::solana_program::instruction::Instruction,
    expected_pubkey: &[u8],
    expected_msg: &[u8],
    expected_sig: &[u8],
) -> Result<()> {
    require!(
        ix.program_id == ed25519_program::ID,
        SignitoError::InvalidVoucherSig
    );

    let data = &ix.data;
    require!(data.len() >= 16, SignitoError::InvalidVoucherSig);
    require!(data[0] == 1, SignitoError::InvalidVoucherSig);

    let h = &data[2..];
    let sig_offset    = u16::from_le_bytes([h[0], h[1]]) as usize;
    let pubkey_offset = u16::from_le_bytes([h[4], h[5]]) as usize;
    let msg_offset    = u16::from_le_bytes([h[8], h[9]]) as usize;
    let msg_size      = u16::from_le_bytes([h[10], h[11]]) as usize;

    require!(data.len() >= sig_offset.saturating_add(64), SignitoError::InvalidVoucherSig);
    require!(data.len() >= pubkey_offset.saturating_add(32), SignitoError::InvalidVoucherSig);
    require!(data.len() >= msg_offset.saturating_add(msg_size), SignitoError::InvalidVoucherSig);
    require!(msg_size == expected_msg.len(), SignitoError::InvalidVoucherSig);

    require!(
        &data[sig_offset..sig_offset + 64] == expected_sig,
        SignitoError::InvalidVoucherSig
    );
    require!(
        &data[pubkey_offset..pubkey_offset + 32] == expected_pubkey,
        SignitoError::InvalidVoucherSig
    );
    require!(
        &data[msg_offset..msg_offset + msg_size] == expected_msg,
        SignitoError::InvalidVoucherSig
    );

    Ok(())
}

#[event]
pub struct VoucherClaimed {
    pub claimer: Pubkey,
    pub issuer: Pubkey,
    pub mint_atoken: Pubkey,
    pub amount: u64,
    pub claimed_at: i64,
}
