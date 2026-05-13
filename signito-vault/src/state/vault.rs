use anchor_lang::prelude::*;

// Vault state stored in PDA: seeds = [b"vault", owner.key()]
//
// OTS chain (from ots.ts):
//   H0  = PBKDF2(vaultCode, walletAddress, 100_000, SHA-256)  -- secret base
//   H1  = SHA-256(H0)
//   ...
//   H32 = SHA-256(H31)  -- stored as current_ots_hash (chain tip)
//
//   Withdrawal 1: reveal H31, program verifies SHA-256(H31) == H32, stores H31 as new tip
//   Withdrawal 2: reveal H30, program verifies SHA-256(H30) == H31, stores H30 as new tip
//   ...
//   Withdrawal 32: reveal H0, program verifies SHA-256(H0) == H1, chain_depth becomes 0
//
// After chain_depth reaches 0, call refresh_ots with a new tip derived from the same
// vault code using the next generation suffix: PBKDF2(vaultCode, wallet + ":gen:" + N).
// This resets the chain without closing the vault or moving funds.
#[account]
pub struct VaultState {
    // Wallet that owns and controls this vault.
    pub owner: Pubkey,
    // Current OTS chain tip. On each withdrawal this advances one step toward H0.
    pub current_ots_hash: [u8; 32],
    // Remaining withdrawal count. Starts at chain_depth passed to initialize_vault.
    pub chain_depth: u8,
    // The NonTransferable Token-2022 mint created for this vault (sSOL, sUSDC, etc.).
    pub mint_stoken: Pubkey,
    // Lamports deposited by user (excludes account rent). Tracked to prevent over-withdrawal.
    pub total_deposited: u64,
    // PDA bump, stored to avoid recomputing in CPIs.
    pub bump: u8,
}

impl VaultState {
    // 8 discriminator + 32 owner + 32 ots_hash + 1 chain_depth
    //   + 32 mint + 8 deposited + 1 bump
    pub const LEN: usize = 8 + 32 + 32 + 1 + 32 + 8 + 1;
}

// Created per-nonce when a voucher is claimed.
// Existence of this account means the nonce was already used.
// Seeds: [b"nonce", mint_atoken.key(), nonce_bytes (16)]
#[account]
pub struct NonceRecord {
    pub claimed_at: i64,
}

impl NonceRecord {
    pub const LEN: usize = 8 + 8;
}
