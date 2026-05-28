import hre from "hardhat";

async function main() {
  const [deployer] = await hre.ethers.getSigners();
  const relayerAddress = deployer.address;

  console.log("Network:", hre.network.name);
  console.log("Deployer / Relayer:", deployer.address);

  const balance = await hre.ethers.provider.getBalance(deployer.address);
  console.log("Deployer balance:", hre.ethers.formatEther(balance), "ETH");

  if (balance === 0n) {
    throw new Error("Deployer has zero balance. Fund the address first.");
  }

  console.log("\n[1/3] Deploying ShieldedETH...");
  const ShieldedETH = await hre.ethers.getContractFactory("ShieldedETH");
  const shETH = await ShieldedETH.deploy();
  await shETH.waitForDeployment();
  const shETHAddress = await shETH.getAddress();
  console.log("ShieldedETH:", shETHAddress);

  console.log("\n[2/3] Deploying SignitoPool...");
  const SignitoPool = await hre.ethers.getContractFactory("SignitoPool");
  const pool = await SignitoPool.deploy(shETHAddress, relayerAddress);
  await pool.waitForDeployment();
  const poolAddress = await pool.getAddress();
  console.log("SignitoPool:", poolAddress);

  console.log("\n[3/3] Wiring shETH.setPool...");
  const wireTx = await shETH.setPool(poolAddress);
  await wireTx.wait();
  console.log("Done.");

  console.log("\n--- Copy these into environment secrets ---");
  console.log(`BASE_SHETH_ADDRESS=${shETHAddress}`);
  console.log(`BASE_POOL_ADDRESS=${poolAddress}`);
  console.log("-------------------------------------------");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
