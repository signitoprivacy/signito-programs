import hre from "hardhat";

async function main() {
  const shETHAddress = process.env.BASE_SHETH_ADDRESS;
  const poolAddress = process.env.BASE_POOL_ADDRESS;
  const relayerAddress = process.env.BASE_RELAYER_ADDRESS;

  if (!shETHAddress || !poolAddress || !relayerAddress) {
    throw new Error("Set BASE_SHETH_ADDRESS, BASE_POOL_ADDRESS, BASE_RELAYER_ADDRESS env vars.");
  }

  console.log("Verifying ShieldedETH:", shETHAddress);
  await hre.run("verify:verify", {
    address: shETHAddress,
    constructorArguments: [],
  });

  console.log("Verifying SignitoPool:", poolAddress);
  await hre.run("verify:verify", {
    address: poolAddress,
    constructorArguments: [shETHAddress, relayerAddress],
  });

  console.log("Verification complete.");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
