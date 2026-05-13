# signito-programs

Signito Vault — on-chain Anchor/Rust program powering the Signito privacy protocol on Solana.

**Program ID:** `9PibgJMUa3zXVd7YWJEJ8UQ14A7z2J3qZ7QDvRW38XeD`  
**Token Standard:** SPL Token-2022 with NonTransferable extension  
**Framework:** Anchor 0.31.1  
**Website:** [signito.org](https://signito.org)  
**Docs:** [docs.signito.org](https://docs.signito.org)

---

## Features

| Feature | Description |
|---|---|
| **SafeVault** | OTS (One-Time Signature) hash-chain vault. Mint sSOL, shield SOL, withdraw with preimage reveals. |
| **StealthSend** | Commitment-nullifier pool. Deposit anonymously, withdraw to a fresh address with no on-chain link. |
| **AirSign** | Ed25519-signed offline vouchers. Issue, share via QR/NFC, claim without internet. |

---

## Program Structure

```
signito-vault/
  src/
    instructions/
      initialize_vault.rs   Create vault PDA, mint sSOL (Token-2022 + NonTransferable)
      deposit.rs            Thaw sSOL account, mint additional sSOL, re-freeze
      unshield.rs           OTS verify, burn sSOL, release SOL to destination
      refresh_ots.rs        Reset OTS chain tip on existing vault
      convert_to_airtoken.rs  Convert sSOL → aSOL for AirSign vouchers
      claim_voucher.rs      Ed25519 verify via instructions sysvar, burn aSOL, release SOL
      close_vault.rs        Close empty vault, reclaim rent
    state/
      vault.rs              VaultState account struct
    errors.rs               SignitoError enum
    constants.rs            Program constants
    lib.rs                  Entry point and instruction dispatch
Anchor.toml
Cargo.toml
Cargo.lock
DEPLOY.md                   Full build and deployment guide
```

---

## SafeVault: OTS Hash Chain

```
H0  = PBKDF2(vaultCode, walletAddress, 100_000 iterations, SHA-256)
H1  = SHA-256(H0)
H2  = SHA-256(H1)
...
H32 = SHA-256(H31)   ← stored on-chain as current_ots_hash (chain tip)

Withdrawal 1: client reveals H31
  Program verifies: SHA-256(H31) == H32
  Program updates tip → H31, decrements chain_depth

Withdrawal 2: client reveals H30
  Program verifies: SHA-256(H30) == H31
  ...continues until chain_depth reaches 0
```

The vault code (passphrase) **never leaves the browser**. Only the PBKDF2-derived hash tip is stored on-chain.

---

## Prerequisites

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Solana CLI 1.18+
sh -c "$(curl -sSfL https://release.solana.com/v1.18.26/install)"

# Anchor CLI 0.31.1
cargo install --git https://github.com/coral-xyz/anchor avm --locked
avm install 0.31.1 && avm use 0.31.1
```

## Build & Test

```bash
pnpm install
anchor build
solana-test-validator --reset &
anchor test --skip-local-validator
```

## Deploy to Devnet

```bash
solana config set --url https://api.devnet.solana.com
solana airdrop 2
anchor build
anchor deploy
```

See [DEPLOY.md](./DEPLOY.md) for full mainnet deployment instructions.

---

## VaultState Account

| Field | Type | Description |
|---|---|---|
| `owner` | Pubkey | Vault owner wallet |
| `current_ots_hash` | [u8; 32] | Current OTS chain tip H_n |
| `chain_depth` | u32 | Remaining withdrawal uses |
| `mint_stoken` | Pubkey | sSOL Token-2022 mint |
| `total_deposited` | u64 | Lamports held in vault PDA |
| `bump` | u8 | PDA bump seed |

---

## Related Repositories

| Repo | Description |
|---|---|
| [signito-app](https://github.com/signitoprivacy/signito-app) | Shield dApp frontend |
| [signito-api](https://github.com/signitoprivacy/signito-api) | Backend API server |
| [signito-docs](https://github.com/signitoprivacy/signito-docs) | Protocol documentation |

---

## License

MIT
