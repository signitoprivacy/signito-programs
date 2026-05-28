# signito-programs-devnet

**Website:** [signito.org](https://signito.org)

Base chain EVM smart contracts powering the Signito privacy protocol.

**Network:** Base Sepolia (chain ID 84532)
**ShieldedETH (sETH):** `0x0f497a7c81608A7dc071E2703813CE1811538D33`
**SignitoPool:** `0xd344786B38bac0B644ff14D74a9822aAD761642e`
**Relayer:** `0xf70494e69aE7090dB21179d2412D76566959B43c`
**Framework:** Hardhat, Solidity 0.8.24

---

## Privacy Model

- `shield()` mints sETH to both `stokenAddress` (random derived) AND `msg.sender` (user wallet)
- `batchAdminMint` adds 20 phantom decoy addresses
- Total: **22 addresses** hold sETH after each shield
- `burnAndQueue` receives a shuffled array of all 22 -- no explicit stokenAddress param
- Contract identifies the real account by OTS preimage match inside the loop
- Observer sees 22 identical-looking addresses with no positional indicator

## Contracts

### ShieldedETH (sETH)
Non-transferable ERC-20 backed 1:1 by ETH. Only pool can mint or burn. Mirrors SPL Token-2022 NonTransferable + PermanentDelegate from the Solana program.

### SignitoPool
Privacy pool with SafeVault, DecoyMix, and AirSign features.

| Function | Description |
|---|---|
| `shield()` | Deposit ETH, mint sETH to stokenAddress + msg.sender |
| `batchAdminMint()` | Relayer mints 20 phantom sETH accounts |
| `burnAndQueue()` | Burn all 22 accounts in shuffled order, queue payout |
| `processQueue()` | Separate TX: send ETH to recipient (no link to burn TX) |
| `refreshOts()` | Rotate OTS hash chain |
| `mintAirsign()` | Burn sETH into ECDSA-keyed offline voucher |
| `claimAirsign()` | Verify eth_personal_sign, release ETH |

## Build

```sh
npm install
npx hardhat compile
npx hardhat run scripts/deploy.ts --network base-sepolia
```

## License

MIT
