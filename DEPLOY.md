# Signito Vault: Build and Deploy Guide

## Prerequisites

Install these tools on your local machine (only required for local builds):

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Solana CLI (1.18+)
sh -c "$(curl -sSfL https://release.solana.com/v1.18.26/install)"

# Anchor CLI (0.31.1)
cargo install --git https://github.com/coral-xyz/anchor avm --locked
avm install 0.31.1
avm use 0.31.1
```

## Setup

```bash
# Enter the programs workspace
cd programs

# Install test dependencies
pnpm install    # or: npm install

# Set up a Solana keypair (skip if you already have ~/.config/solana/id.json)
solana-keygen new
```

## Local Test (Localnet)

```bash
# Start local test validator in one terminal
solana-test-validator --reset

# In another terminal, build and test
cd programs
anchor build
anchor test --skip-local-validator
```

## Devnet Deploy

```bash
# Switch to devnet
solana config set --url https://api.devnet.solana.com

# Airdrop SOL for deploy fees (devnet only)
solana airdrop 2

# Build the program
anchor build

# Get the program ID from the keypair
solana address -k target/deploy/signito_vault-keypair.json

# Update the ID in two places:
# 1. programs/Anchor.toml -> [programs.localnet] and [programs.mainnet]
# 2. programs/signito-vault/src/lib.rs -> declare_id!(...)

# Rebuild with correct ID
anchor build

# Deploy (~0.57 SOL buffer needed, ~0.29 SOL permanent on devnet)
anchor deploy
```

## Mainnet Deploy

```bash
# Switch to mainnet
solana config set --url https://api.mainnet-beta.solana.com

# Ensure wallet has enough SOL:
# - Program rent: ~0.29 SOL per 40KB (permanent)
# - Deploy buffer: ~0.57 SOL (refunded after deploy)
# - Transaction fees: ~0.01 SOL

# Deploy with mainnet cluster
anchor deploy --provider.cluster mainnet

# Update Anchor.toml [programs.mainnet] with the real program ID
# Update lib/api-spec/openapi.yaml PROGRAM_ID reference
```

## OTS Chain Design

The OTS (One-Time Signature) hash chain for the vault works as follows:

```
H0  = PBKDF2(vaultCode, walletAddress, 100_000 iterations, SHA-256)
H1  = SHA-256(H0)
H2  = SHA-256(H1)
...
H32 = SHA-256(H31)   <-- stored on-chain as current_ots_hash (chain tip)

Withdrawal 1: client reveals H31
  - Program verifies: SHA-256(H31) == H32 (stored tip)
  - Program updates tip to H31 and decrements chain_depth

Withdrawal 2: client reveals H30
  - Program verifies: SHA-256(H30) == H31 (new tip)
  - ...continues until chain_depth reaches 0

After 32 withdrawals: vault is exhausted. Close and create a new vault.
```

The client-side derivation is in `artifacts/shield-app/src/lib/ots.ts`.
The on-chain verification is in `programs/signito-vault/src/instructions/unshield.rs`.

Both must use the same hash function: `SHA-256` (NOT PBKDF2 for the chain steps).

## Rent Recovery

To recover rent when a program is no longer needed:

```bash
# Close a vault account (user must have zero deposited first)
# This is handled by the close_vault instruction (user calls it from the UI)

# To close the program itself and recover rent (use only when decommissioning):
solana program close <PROGRAM_ID> --bypass-warning
# WARNING: This makes the program non-executable. Do NOT use --final.
```

## Account Sizes and Rent

| Account | Size (bytes) | Rent (SOL) |
|---|---|---|
| VaultState PDA | 114 | ~0.00194 |
| sSOL Mint (Token-2022 + NonTransferable) | ~170 | ~0.00290 |
| ATA (Token-2022) | ~170 | ~0.00290 |
| NonceRecord PDA | 16 | ~0.00142 |

Formula: `lamports = (bytes + 128) * 6960`

## Environment Variables (API Server)

Set in your deployment environment:

| Variable | Description |
|---|---|
| HELIUS_API_KEY | Helius RPC key (get free at helius.dev) |
| DATABASE_URL | PostgreSQL for vault metadata |
| SESSION_SECRET | Express session secret |
