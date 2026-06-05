import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha512";
import { blake3 } from "@noble/hashes/blake3";
import type { Command, PartyId, SignedCommand } from "./types.js";
import { signingBytes, signingBytesForChain } from "./encoding.js";

// @noble/ed25519 v2 needs sha512 wired up for synchronous signing.
ed.etc.sha512Sync = (...m: Uint8Array[]) => sha512(ed.etc.concatBytes(...m));

/**
 * A party's signing identity. Mirrors the Rust `PartyIdentity`:
 * the 32-byte seed IS the Ed25519 secret key (dalek `from_bytes`).
 */
export class PartyIdentity {
  readonly party: PartyId;
  private readonly secret: Uint8Array; // 32-byte seed

  constructor(party: PartyId, seed: Uint8Array) {
    if (seed.length !== 32) throw new Error("seed must be 32 bytes");
    this.party = party;
    this.secret = seed;
  }

  /** Deterministic identity from a label + raw 32-byte seed. */
  static fromSeed(party: PartyId, seed: Uint8Array): PartyIdentity {
    return new PartyIdentity(party, seed);
  }

  /**
   * Convenience: derive a 32-byte seed from a passphrase via BLAKE3.
   * (Use real entropy in production; this mirrors test/dev helpers.)
   */
  static fromPassphrase(party: PartyId, passphrase: string): PartyIdentity {
    const seed = blake3(new TextEncoder().encode(passphrase));
    return new PartyIdentity(party, seed);
  }

  /** Generate a random identity. */
  static generate(party: PartyId): PartyIdentity {
    return new PartyIdentity(party, ed.utils.randomPrivateKey());
  }

  /** The 32-byte Ed25519 public key. */
  publicKey(): Uint8Array {
    return ed.getPublicKey(this.secret);
  }

  /** Sign a command, producing a SignedCommand ready for submission. */
  sign(command: Command): SignedCommand {
    return this.signForChain(command, 0);
  }

  /**
   * Sign a command bound to a specific chain id (replay protection). Use the
   * chain's `chain_id` (from `veilux_chainId` / genesis); 0 = legacy/dev.
   */
  signForChain(command: Command, chainId: number): SignedCommand {
    const msg = signingBytesForChain(command, chainId);
    const signature = ed.sign(msg, this.secret);
    return {
      command,
      public_key: Array.from(this.publicKey()),
      signature: Array.from(signature),
      chain_id: chainId,
    };
  }
}
