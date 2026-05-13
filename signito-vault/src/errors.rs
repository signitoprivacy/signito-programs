use anchor_lang::prelude::*;

#[error_code]
pub enum SignitoError {
    #[msg("OTS preimage does not match. Vault code incorrect or wrong withdrawal step.")]
    InvalidOtsPreimage,

    #[msg("Vault chain exhausted. All OTS levels consumed. Create a new vault.")]
    VaultExhausted,

    #[msg("Requested amount exceeds vault balance.")]
    InsufficientFunds,

    #[msg("Amount must be greater than zero.")]
    InvalidAmount,

    #[msg("Ed25519 signature verification failed. Voucher is invalid.")]
    InvalidVoucherSig,

    #[msg("This voucher has expired.")]
    VoucherExpired,

    #[msg("This voucher has already been claimed.")]
    VoucherAlreadyClaimed,

    #[msg("This voucher was issued to a different address.")]
    RecipientMismatch,

    #[msg("Vault still holds funds. Withdraw all SOL before closing.")]
    VaultNotEmpty,

    #[msg("Unauthorized. Only the designated admin can perform this action.")]
    Unauthorized,

    #[msg("Arithmetic overflow.")]
    Overflow,
}
