#!/usr/bin/env bash
# Signito Vault: Devnet deployment script
# Run from project root: bash programs/deploy-devnet.sh

set -e

export PATH="/home/runner/.cargo/bin:/home/runner/.local/bin:/home/runner/.local/share/solana/install/active_release/bin:$PATH"

PROGRAM_SO="programs/target/deploy/signito_vault.so"
PROGRAM_KEYPAIR="programs/target/deploy/signito_vault-keypair.json"
DEPLOYER_KEYPAIR="programs/deployer-keypair.json"
RPC="https://api.devnet.solana.com"

echo "=== Signito Vault: Devnet Deploy ==="
echo ""

# Verify binary exists
if [ ! -f "$PROGRAM_SO" ]; then
  echo "ERROR: Program binary not found at $PROGRAM_SO"
  echo "Run: cd programs && anchor build"
  exit 1
fi

PROGRAM_ID=$(solana-keygen pubkey "$PROGRAM_KEYPAIR")
DEPLOYER=$(solana-keygen pubkey "$DEPLOYER_KEYPAIR")

echo "Program ID  : $PROGRAM_ID"
echo "Deployer    : $DEPLOYER"
echo "Binary size : $(du -h $PROGRAM_SO | cut -f1)"
echo "RPC         : $RPC"
echo ""

# Check deployer balance
BALANCE=$(solana balance "$DEPLOYER" --url "$RPC" 2>/dev/null | awk '{print $1}')
echo "Deployer balance: ${BALANCE} SOL"
echo ""

if (( $(echo "$BALANCE < 1.0" | bc -l 2>/dev/null || echo "1") )); then
  echo "WARN: Deployer balance may be too low. Need at least 1 SOL."
  echo "Fund: $DEPLOYER"
  echo "Faucet: https://faucet.solana.com"
  echo ""
fi

echo "Deploying program..."
solana program deploy \
  "$PROGRAM_SO" \
  --program-id "$PROGRAM_KEYPAIR" \
  --keypair "$DEPLOYER_KEYPAIR" \
  --url "$RPC" \
  --commitment confirmed

echo ""
echo "Deploy complete!"
echo "Program ID: $PROGRAM_ID"
echo "Explorer  : https://explorer.solana.com/address/$PROGRAM_ID?cluster=devnet"
