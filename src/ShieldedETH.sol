// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

// ShieldedETH (sETH) -- non-transferable ERC-20 backed 1:1 by ETH in SignitoPool.
//
// Privacy model:
//   - Users receive sETH at a random stokenAddress (not their wallet).
//   - sETH cannot be transferred: it can only be minted (shield) or burned (unshield).
//   - Only the authorized pool contract can mint or burn.
//   - This replicates Token-2022 NonTransferable + PermanentDelegate from Solana.
contract ShieldedETH is ERC20 {
    address public pool;
    address public owner;

    event PoolSet(address indexed pool);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier onlyPool() {
        require(msg.sender == pool, "not pool");
        _;
    }

    constructor() ERC20("Shielded ETH", "sETH") {
        owner = msg.sender;
    }

    function setPool(address _pool) external onlyOwner {
        require(_pool != address(0), "zero address");
        pool = _pool;
        emit PoolSet(_pool);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "zero address");
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    function mint(address to, uint256 amount) external onlyPool {
        _mint(to, amount);
    }

    function burn(address from, uint256 amount) external onlyPool {
        _burn(from, amount);
    }

    // Non-transferable: only minting (from == address(0)) and burning (to == address(0)) are allowed.
    // Direct transfers and approvals are disabled.
    function _update(address from, address to, uint256 value) internal override {
        require(
            from == address(0) || to == address(0),
            "sETH: non-transferable"
        );
        super._update(from, to, value);
    }
}
