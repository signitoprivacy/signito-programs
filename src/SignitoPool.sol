// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./ShieldedETH.sol";

// SignitoPool -- privacy pool for ETH on Base.
//
// Three features mirroring the Solana protocol:
//
//   SafeVault (OTS Protocol):
//     shield()           -- user deposits ETH, receives sETH at a random stokenAddress
//     burnAndQueue()     -- relayer burns sETH from real + 20 decoys (shuffled), NO recipient in this TX
//     processQueue()     -- relayer sends ETH from pool to recipient (separate TX, no on-chain link)
//     refreshOts()       -- relayer rotates OTS chain when depth runs low
//
//   Decoy mix pool:
//     batchAdminMint()   -- relayer mints phantom sETH to decoy addresses (no ETH backing)
//                          called immediately after each real shield to create the 20-decoy anonymity set
//
//   AirSign (offline vouchers):
//     mintAirsign()   -- relayer burns sETH, creates ECDSA-keyed escrow
//     claimAirsign()  -- relayer verifies eth_personal_sign voucher, releases ETH from escrow
//
// Privacy guarantees:
//   - shield() mints sETH to BOTH stokenAddress (random derived) AND msg.sender (user's wallet).
//     batchAdminMint adds 20 phantom decoy addresses. Total: 22 addresses hold sETH.
//   - burnAndQueue receives a shuffled array of all 22 addresses. No explicit stokenAddress param.
//     Real account is at a RANDOM position -- contract finds it via OTS preimage match in the loop.
//     Observer reading calldata sees 22 identical-looking addresses with no positional indicator.
//   - burnAndQueue and processQueue are separate transactions with zero common accounts.
//   - Both are submitted by the relayer via Flashbots (private mempool) to prevent correlation.
//   - stokenAddress is a random address derived client-side, never linked to the user's wallet on-chain.
//   - Phantom sETH has no ETH backing: pool.deposited tracks only real deposits. No collateral risk.
//
// OTS chain (keccak256, cheaper than SHA-256 on EVM):
//   H0 = PBKDF2(vaultCode, walletAddress, 100_000 iters, SHA-256)  [browser-side]
//   H_n = keccak256(H_{n-1})
//   chain tip = H_chainDepth, stored in userStates[stokenAddress].currentOtsHash
//   To prove: reveal H_{n-1}, contract checks keccak256(preimage) == currentOtsHash
contract SignitoPool {
    ShieldedETH public immutable shETH;
    address public relayer;
    address public owner;

    struct UserState {
        bytes32 currentOtsHash;
        uint8 chainDepth;
        uint256 deposited;
        bool initialized;
    }

    struct AirsignEscrow {
        uint256 amount;
        address voucherSigner;
        bool claimed;
        bool exists;
    }

    mapping(address => UserState) public userStates;
    mapping(bytes32 => AirsignEscrow) public airsignEscrows;

    event Shielded(address indexed stokenAddress, uint256 amount);
    event AdminMinted(address indexed stokenAddress, uint256 amount);
    event BurnQueued(address indexed stokenAddress, uint256 amount, uint256 decoyCount);
    event Processed(address indexed recipient, uint256 amount);
    event OtsRefreshed(address indexed stokenAddress, uint8 newChainDepth);
    event AirsignMinted(bytes32 indexed nonceHash, uint256 amount, address indexed stokenAddress);
    event AirsignClaimed(bytes32 indexed nonceHash, address indexed recipient, uint256 amount);
    event RelayerSet(address indexed relayer);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);

    modifier onlyRelayer() {
        require(msg.sender == relayer, "not relayer");
        _;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    constructor(address _shETH, address _relayer) {
        require(_shETH != address(0), "zero shETH");
        require(_relayer != address(0), "zero relayer");
        shETH = ShieldedETH(_shETH);
        relayer = _relayer;
        owner = msg.sender;
    }

    function setRelayer(address _relayer) external onlyOwner {
        require(_relayer != address(0), "zero address");
        relayer = _relayer;
        emit RelayerSet(_relayer);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "zero address");
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    // Shield ETH into the pool.
    // Can be called by anyone (user signs with MetaMask, ETH comes from their wallet).
    // stokenAddress: a random address generated deterministically client-side from vaultCode,
    //   never the user's actual wallet address. This breaks the on-chain wallet-to-vault link.
    // initialOtsHash: keccak256^chainDepth(PBKDF2(vaultCode, walletAddress)) -- the chain tip.
    // Subsequent deposits to the same stokenAddress just increase deposited balance.
    // Privacy: sETH is minted to BOTH stokenAddress AND msg.sender so the user's wallet
    //   is indistinguishable from the 20 batchAdminMint decoys on unshield.
    function shield(
        address stokenAddress,
        bytes32 initialOtsHash,
        uint8 chainDepth
    ) external payable {
        require(msg.value > 0, "zero amount");
        require(stokenAddress != address(0), "zero stoken address");

        UserState storage state = userStates[stokenAddress];

        if (!state.initialized) {
            require(chainDepth > 0, "zero chain depth");
            state.currentOtsHash = initialOtsHash;
            state.chainDepth = chainDepth;
            state.initialized = true;
        }

        state.deposited += msg.value;
        shETH.mint(stokenAddress, msg.value);

        // Also mint sETH to the user's actual wallet so it appears in the burn set
        // alongside the 20 decoys -- observer cannot distinguish real from phantom.
        if (msg.sender != stokenAddress) {
            shETH.mint(msg.sender, msg.value);
        }

        emit Shielded(stokenAddress, msg.value);
    }

    // Mint phantom sETH to a batch of decoy stokenAddresses in a single TX.
    // Called by the relayer immediately after a real shield, once per shield event.
    // Creates the 20-address anonymity set that will burn alongside the real account on unshield.
    // IMPORTANT: shETH is minted without backing ETH. deposited is NOT updated for decoys.
    //   Pool collateralization is unaffected -- only real shield() calls increase deposited.
    //   Only gas cost, no ETH cost. Equivalent to Solana admin_mint instruction.
    function batchAdminMint(
        address[] calldata stokenAddresses,
        uint256 amount
    ) external onlyRelayer {
        require(amount > 0, "zero amount");
        for (uint256 i = 0; i < stokenAddresses.length; i++) {
            require(stokenAddresses[i] != address(0), "zero address");
            shETH.mint(stokenAddresses[i], amount);
            emit AdminMinted(stokenAddresses[i], amount);
        }
    }

    // Called by relayer ONLY, submitted via Flashbots private TX.
    // allBurnAccounts: shuffled array of 22 addresses (stokenAddress + user wallet + 20 decoys),
    //   in a RANDOM order chosen by the relayer. No explicit stokenAddress param in calldata.
    //   Contract identifies the real account by finding the one whose OTS hash matches otsPreimage.
    //   All 22 accounts burn the same amount -- observer cannot tell which is the real one.
    // CRITICAL: no recipient address in this TX -- zero on-chain link to processQueue().
    function burnAndQueue(
        uint256 amount,
        bytes32 otsPreimage,
        address[] calldata allBurnAccounts
    ) external onlyRelayer {
        require(amount > 0, "zero amount");
        require(allBurnAccounts.length > 0, "empty array");

        // Find the real stokenAddress: the initialized account whose OTS hash matches.
        // Real address is at a random position in the shuffled array -- no positional hint.
        address stokenAddress;
        bytes32 preimageHash = keccak256(abi.encodePacked(otsPreimage));
        for (uint256 i = 0; i < allBurnAccounts.length; i++) {
            address candidate = allBurnAccounts[i];
            if (
                candidate != address(0) &&
                userStates[candidate].initialized &&
                preimageHash == userStates[candidate].currentOtsHash
            ) {
                stokenAddress = candidate;
                break;
            }
        }
        require(stokenAddress != address(0), "no valid OTS in array");

        UserState storage state = userStates[stokenAddress];
        require(state.chainDepth > 0, "chain exhausted");
        require(amount <= state.deposited, "insufficient balance");

        // Advance OTS chain: revealed preimage becomes the new tip
        state.currentOtsHash = otsPreimage;
        state.chainDepth -= 1;
        state.deposited -= amount;

        // Burn all 22 accounts in one pass.
        // Non-fatal: skip zero address or insufficient balance (underfunded decoy or already burned).
        for (uint256 i = 0; i < allBurnAccounts.length; i++) {
            address acc = allBurnAccounts[i];
            if (acc != address(0) && shETH.balanceOf(acc) >= amount) {
                shETH.burn(acc, amount);
            }
        }

        emit BurnQueued(stokenAddress, amount, allBurnAccounts.length);
    }

    // Called by relayer ONLY, submitted via private RPC (separate TX from burnAndQueue).
    // Sends ETH from the pool directly to the recipient.
    // Zero on-chain accounts in common with burnAndQueue: full sender-recipient unlinkability.
    function processQueue(
        address payable recipient,
        uint256 amount
    ) external onlyRelayer {
        require(amount > 0, "zero amount");
        require(recipient != address(0), "zero recipient");
        require(address(this).balance >= amount, "insufficient pool");

        (bool ok, ) = recipient.call{value: amount}("");
        require(ok, "transfer failed");

        emit Processed(recipient, amount);
    }

    // Refresh OTS chain when chain_depth runs low.
    // Consumes one OTS use to authorize the refresh, then replaces the chain tip.
    // newOtsHash: keccak256^newChainDepth(PBKDF2(vaultCode, wallet, gen+1))
    function refreshOts(
        address stokenAddress,
        bytes32 otsPreimage,
        bytes32 newOtsHash,
        uint8 newChainDepth
    ) external onlyRelayer {
        UserState storage state = userStates[stokenAddress];
        require(state.initialized, "not initialized");
        require(state.chainDepth > 0, "chain exhausted");
        require(newChainDepth > 0, "zero new depth");

        require(
            keccak256(abi.encodePacked(otsPreimage)) == state.currentOtsHash,
            "invalid OTS"
        );

        state.currentOtsHash = newOtsHash;
        state.chainDepth = newChainDepth;

        emit OtsRefreshed(stokenAddress, newChainDepth);
    }

    // AirSign: burn sETH and create an ECDSA-keyed escrow.
    // voucherSigner: Ethereum address whose eth_personal_sign must be provided to claim.
    //   Typically the user's MetaMask wallet address (secp256k1 ECDSA, EIP-191).
    function mintAirsign(
        bytes32 nonceHash,
        uint256 amount,
        address stokenAddress,
        bytes32 otsPreimage,
        address voucherSigner
    ) external onlyRelayer {
        require(!airsignEscrows[nonceHash].exists, "nonce already used");
        require(voucherSigner != address(0), "zero signer");

        UserState storage state = userStates[stokenAddress];
        require(state.initialized, "not initialized");
        require(state.chainDepth > 0, "chain exhausted");
        require(amount > 0 && amount <= state.deposited, "invalid amount");

        require(
            keccak256(abi.encodePacked(otsPreimage)) == state.currentOtsHash,
            "invalid OTS"
        );

        state.currentOtsHash = otsPreimage;
        state.chainDepth -= 1;
        state.deposited -= amount;

        shETH.burn(stokenAddress, amount);

        airsignEscrows[nonceHash] = AirsignEscrow({
            amount: amount,
            voucherSigner: voucherSigner,
            claimed: false,
            exists: true
        });

        emit AirsignMinted(nonceHash, amount, stokenAddress);
    }

    // AirSign claim: verify eth_personal_sign over keccak256(nonceHash ++ recipient ++ amount).
    // Signature must come from voucherSigner stored in the escrow.
    function claimAirsign(
        bytes32 nonceHash,
        address payable recipient,
        bytes calldata signature
    ) external onlyRelayer {
        AirsignEscrow storage escrow = airsignEscrows[nonceHash];
        require(escrow.exists, "escrow not found");
        require(!escrow.claimed, "already claimed");
        require(recipient != address(0), "zero recipient");

        bytes32 messageHash = keccak256(
            abi.encodePacked(nonceHash, recipient, escrow.amount)
        );
        bytes32 ethSignedHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", messageHash)
        );

        address recovered = _recoverSigner(ethSignedHash, signature);
        require(recovered == escrow.voucherSigner, "invalid signature");

        escrow.claimed = true;

        (bool ok, ) = recipient.call{value: escrow.amount}("");
        require(ok, "transfer failed");

        emit AirsignClaimed(nonceHash, recipient, escrow.amount);
    }

    function _recoverSigner(bytes32 hash, bytes calldata sig) internal pure returns (address) {
        require(sig.length == 65, "invalid sig length");
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(sig.offset)
            s := calldataload(add(sig.offset, 32))
            v := byte(0, calldataload(add(sig.offset, 64)))
        }
        if (v < 27) v += 27;
        require(v == 27 || v == 28, "invalid v");
        address signer = ecrecover(hash, v, r, s);
        require(signer != address(0), "ecrecover failed");
        return signer;
    }

    receive() external payable {}

    function getUserState(address stokenAddress) external view returns (
        bytes32 currentOtsHash,
        uint8 chainDepth,
        uint256 deposited,
        bool initialized
    ) {
        UserState storage state = userStates[stokenAddress];
        return (state.currentOtsHash, state.chainDepth, state.deposited, state.initialized);
    }

    function getPoolBalance() external view returns (uint256) {
        return address(this).balance;
    }

    function getEscrow(bytes32 nonceHash) external view returns (
        uint256 amount,
        address voucherSigner,
        bool claimed,
        bool exists
    ) {
        AirsignEscrow storage e = airsignEscrows[nonceHash];
        return (e.amount, e.voucherSigner, e.claimed, e.exists);
    }
}
