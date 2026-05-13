import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import { SignitoVault } from "../target/types/signito_vault";
import {
  PublicKey,
  Keypair,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Ed25519Program,
  Transaction,
} from "@solana/web3.js";
import {
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
} from "@solana/spl-token";
import { assert } from "chai";
import * as crypto from "crypto";

// Mirror of ots.ts PBKDF2 hash chain logic (Node.js version for tests)
async function derivePbkdf2(password: string, salt: string): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    crypto.pbkdf2(password, salt, 100_000, 32, "sha256", (err, key) => {
      if (err) reject(err);
      else resolve(key);
    });
  });
}

function sha256(input: Buffer): Buffer {
  return crypto.createHash("sha256").update(input).digest();
}

async function deriveOtsTip(vaultCode: string, wallet: string, chainDepth = 32): Promise<Buffer> {
  let current = await derivePbkdf2(vaultCode, wallet);
  for (let i = 0; i < chainDepth; i++) {
    current = sha256(current);
  }
  return current;
}

async function deriveOtsPreimage(
  vaultCode: string,
  wallet: string,
  chainDepth: number,
  step: number
): Promise<Buffer> {
  let current = await derivePbkdf2(vaultCode, wallet);
  const targetDepth = chainDepth - step;
  for (let i = 0; i < targetDepth; i++) {
    current = sha256(current);
  }
  return current;
}

describe("signito_vault", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.SignitoVault as Program<SignitoVault>;
  const connection = provider.connection;

  let owner: Keypair;
  let mintStoken: Keypair;
  let vaultPda: PublicKey;
  let vaultBump: number;
  let ownerStokenAta: PublicKey;

  const VAULT_CODE = "AB12cd34"; // 4 letters + 4 digits
  const CHAIN_DEPTH = 32;
  const DEPOSIT_LAMPORTS = new BN(500_000_000); // 0.5 SOL

  before(async () => {
    owner = Keypair.generate();
    mintStoken = Keypair.generate();

    // Airdrop 2 SOL to owner
    const sig = await connection.requestAirdrop(owner.publicKey, 2_000_000_000);
    await connection.confirmTransaction(sig, "confirmed");

    // Derive vault PDA
    [vaultPda, vaultBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), owner.publicKey.toBuffer()],
      program.programId
    );

    // Derive ATA for sSOL (Token-2022)
    ownerStokenAta = getAssociatedTokenAddressSync(
      mintStoken.publicKey,
      owner.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID
    );
  });

  // ---- initialize_vault ----

  describe("initialize_vault", () => {
    it("creates vault PDA with correct OTS tip and mints sSOL", async () => {
      const otsTip = await deriveOtsTip(VAULT_CODE, owner.publicKey.toBase58(), CHAIN_DEPTH);

      await program.methods
        .initializeVault({
          otsTip: Array.from(otsTip),
          chainDepth: CHAIN_DEPTH,
          amount: DEPOSIT_LAMPORTS,
        })
        .accounts({
          owner: owner.publicKey,
          vaultPda,
          mintStoken: mintStoken.publicKey,
          ownerStokenAta,
          systemProgram: SystemProgram.programId,
          tokenProgram2022: TOKEN_2022_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([owner, mintStoken])
        .rpc();

      const vault = await program.account.vaultState.fetch(vaultPda);

      assert.equal(vault.owner.toBase58(), owner.publicKey.toBase58());
      assert.equal(vault.chainDepth, CHAIN_DEPTH);
      assert.equal(vault.mintStoken.toBase58(), mintStoken.publicKey.toBase58());
      assert.equal(vault.totalDeposited.toString(), DEPOSIT_LAMPORTS.toString());
      assert.deepEqual(Array.from(vault.currentOtsHash), Array.from(otsTip));
    });

    it("rejects zero amount", async () => {
      const owner2 = Keypair.generate();
      const mint2 = Keypair.generate();
      await connection.requestAirdrop(owner2.publicKey, 1_000_000_000).then(
        (s) => connection.confirmTransaction(s, "confirmed")
      );
      const [vault2] = PublicKey.findProgramAddressSync(
        [Buffer.from("vault"), owner2.publicKey.toBuffer()],
        program.programId
      );
      const tip = await deriveOtsTip("XY12ab34", owner2.publicKey.toBase58());

      try {
        await program.methods
          .initializeVault({ otsTip: Array.from(tip), chainDepth: 32, amount: new BN(0) })
          .accounts({
            owner: owner2.publicKey,
            vaultPda: vault2,
            mintStoken: mint2.publicKey,
            ownerStokenAta: getAssociatedTokenAddressSync(
              mint2.publicKey, owner2.publicKey, false, TOKEN_2022_PROGRAM_ID
            ),
            systemProgram: SystemProgram.programId,
            tokenProgram2022: TOKEN_2022_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .signers([owner2, mint2])
          .rpc();
        assert.fail("should have thrown");
      } catch (err: any) {
        assert.include(err.toString(), "InvalidAmount");
      }
    });
  });

  // ---- unshield ----

  describe("unshield", () => {
    let destination: Keypair;

    before(() => {
      destination = Keypair.generate(); // fresh address, no prior history
    });

    it("withdraws SOL with correct OTS preimage (step 1)", async () => {
      const preimage = await deriveOtsPreimage(
        VAULT_CODE, owner.publicKey.toBase58(), CHAIN_DEPTH, 1
      );
      const withdrawAmount = new BN(100_000_000); // 0.1 SOL

      const destBefore = await connection.getBalance(destination.publicKey);
      const vaultBefore = await program.account.vaultState.fetch(vaultPda);

      await program.methods
        .unshield({ otsPreimage: Array.from(preimage), amount: withdrawAmount })
        .accounts({
          owner: owner.publicKey,
          vaultPda,
          mintStoken: mintStoken.publicKey,
          ownerStokenAta,
          destination: destination.publicKey,
          tokenProgram2022: TOKEN_2022_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const destAfter = await connection.getBalance(destination.publicKey);
      const vaultAfter = await program.account.vaultState.fetch(vaultPda);

      assert.equal(destAfter - destBefore, withdrawAmount.toNumber());
      assert.equal(
        vaultAfter.totalDeposited.toString(),
        vaultBefore.totalDeposited.sub(withdrawAmount).toString()
      );
      assert.equal(vaultAfter.chainDepth, CHAIN_DEPTH - 1);
      // current_ots_hash should now equal the submitted preimage
      assert.deepEqual(Array.from(vaultAfter.currentOtsHash), Array.from(preimage));
    });

    it("rejects wrong OTS preimage (no state change)", async () => {
      const wrongPreimage = Buffer.alloc(32, 0xff); // clearly wrong

      const vaultBefore = await program.account.vaultState.fetch(vaultPda);

      try {
        await program.methods
          .unshield({ otsPreimage: Array.from(wrongPreimage), amount: new BN(100_000_000) })
          .accounts({
            owner: owner.publicKey,
            vaultPda,
            mintStoken: mintStoken.publicKey,
            ownerStokenAta,
            destination: destination.publicKey,
            tokenProgram2022: TOKEN_2022_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc();
        assert.fail("should have thrown");
      } catch (err: any) {
        assert.include(err.toString(), "InvalidOtsPreimage");
      }

      // Vault state must be unchanged
      const vaultAfter = await program.account.vaultState.fetch(vaultPda);
      assert.equal(vaultAfter.chainDepth, vaultBefore.chainDepth);
      assert.equal(vaultAfter.totalDeposited.toString(), vaultBefore.totalDeposited.toString());
    });

    it("rejects amount exceeding balance", async () => {
      const vault = await program.account.vaultState.fetch(vaultPda);
      const overAmount = new BN(vault.totalDeposited.toNumber() + 1_000_000);
      const step = CHAIN_DEPTH - vault.chainDepth + 1;
      const preimage = await deriveOtsPreimage(
        VAULT_CODE, owner.publicKey.toBase58(), CHAIN_DEPTH, step
      );

      try {
        await program.methods
          .unshield({ otsPreimage: Array.from(preimage), amount: overAmount })
          .accounts({
            owner: owner.publicKey,
            vaultPda,
            mintStoken: mintStoken.publicKey,
            ownerStokenAta,
            destination: destination.publicKey,
            tokenProgram2022: TOKEN_2022_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc();
        assert.fail("should have thrown");
      } catch (err: any) {
        assert.include(err.toString(), "InsufficientFunds");
      }
    });
  });

  // ---- close_vault ----

  describe("close_vault", () => {
    it("rejects closing vault that still has funds", async () => {
      try {
        await program.methods
          .closeVault()
          .accounts({
            owner: owner.publicKey,
            vaultPda,
            systemProgram: SystemProgram.programId,
          })
          .signers([owner])
          .rpc();
        assert.fail("should have thrown");
      } catch (err: any) {
        assert.include(err.toString(), "VaultNotEmpty");
      }
    });

    it("closes empty vault and returns rent to owner", async () => {
      // Drain the vault completely first
      const vault = await program.account.vaultState.fetch(vaultPda);
      const remaining = vault.totalDeposited;
      const step = CHAIN_DEPTH - vault.chainDepth + 1;
      const preimage = await deriveOtsPreimage(
        VAULT_CODE, owner.publicKey.toBase58(), CHAIN_DEPTH, step
      );

      const dest = Keypair.generate();
      await program.methods
        .unshield({ otsPreimage: Array.from(preimage), amount: remaining })
        .accounts({
          owner: owner.publicKey,
          vaultPda,
          mintStoken: mintStoken.publicKey,
          ownerStokenAta,
          destination: dest.publicKey,
          tokenProgram2022: TOKEN_2022_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const ownerBefore = await connection.getBalance(owner.publicKey);

      await program.methods
        .closeVault()
        .accounts({
          owner: owner.publicKey,
          vaultPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();

      const ownerAfter = await connection.getBalance(owner.publicKey);

      // Owner should have received rent back (minus tx fee)
      assert.isAbove(ownerAfter, ownerBefore - 10_000); // 10k lamports tx fee tolerance

      // Vault account should be gone
      const vaultAcc = await connection.getAccountInfo(vaultPda);
      assert.isNull(vaultAcc);
    });
  });
});
