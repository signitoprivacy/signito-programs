# signito-programs-devnet

**Website:** [signito.org](https://signito.org)

Base chain EVM smart contracts powering the Signito privacy protocol.

**Network:** Base Sepolia (chain ID 84532)
**ShieldedETH (sETH):** `0x5e112428697dA966dC1603eA5cB96B71508c3a03`
**SignitoPool:** `0x8C7Eeb11C7c8D58b0d12A772B146313aaAAEaBdb`
**Relayer:** `0xf70494e69aE7090dB21179d2412D76566959B43c`
**Framework:** Hardhat, Solidity 0.8.24

---

## Contracts

### ShieldedETH (sETH)

Non-transferable ERC-20 backed 1:1 by ETH in the pool. Mirrors SPL Token-2022 NonTransferable + PermanentDelegate from the Solana program.

- Minted when a user shields ETH
- Burned when a user unshields ETH
- Cannot be transferred -- only the pool contract can mint or burn

### SignitoPool

Privacy pool contract implementing the same three-feature architecture as the Solana program.

| Function | Description |
|---|---|
| `shield()` | User deposits ETH, receives sETH at a random stokenAddress |
| `batchAdminMint()` | Relayer mints phantom sETH to 20 decoy addresses (no ETH backing) |
| `burnAndQueue()` | Relayer burns real + 20 decoy sETH simultaneously, no recipient in TX |
| `processQueue()` | Relayer sends ETH to recipient in a separate TX (zero on-chain link) |
| `refreshOts()` | Rotate the OTS hash chain |
| `mintAirsign()` | Burn sETH into an ECDSA-keyed offline voucher |
| `claimAirsign()` | Verify eth_personal_sign and release ETH |

## Privacy Model

- **burnAndQueue** and **processQueue** are separate transactions with zero accounts in common
- Both are submitted via Flashbots Protect on mainnet (private mempool)
- OTS preimage is verified off-chain by the API server
- stokenAddress is a random address derived client-side, never linked to the user's wallet
- **Decoy mix layer**: 20 phantom sETH accounts are minted via `batchAdminMint` after every real shield. When the user unshields, all 21 accounts burn in the same transaction. Observer cannot identify the real account.

## OTS Chain

```
H0 = PBKDF2(vaultCode, walletAddress, 100_000 iters, SHA-256)
H_n = keccak256(H_{n-1})
chain tip = H_chainDepth, stored in userStates[stokenAddress].currentOtsHash
To prove: reveal H_{n-1}, contract checks keccak256(preimage) == currentOtsHash
```

## Build and Deploy

```sh
npm install
npx hardhat compile
npx hardhat run scripts/deploy.ts --network base-sepolia
```

## License

MIT
