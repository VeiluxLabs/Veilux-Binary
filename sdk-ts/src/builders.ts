import type { Command, PartyId, Visibility } from "./types.js";

const enc = new TextEncoder();

/** Encode a prism command object as the kernel `payload` byte array. */
function payloadOf(obj: unknown): number[] {
  return Array.from(enc.encode(JSON.stringify(obj)));
}

/**
 * Convert a `0x`-prefixed (or bare) hex hash to a 32-number array, because the
 * Rust `Hash(pub [u8;32])` newtype serializes as a JSON array of bytes — not a
 * hex string. Hash-typed command fields must use this.
 */
function hashBytes(hex: string): number[] {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (clean.length !== 64) throw new Error(`hash must be 32 bytes (64 hex chars), got ${clean.length}`);
  const out: number[] = [];
  for (let i = 0; i < 64; i += 2) out.push(parseInt(clean.slice(i, i + 2), 16));
  return out;
}

function cmd(
  prism: string,
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  payloadObj: unknown,
): Command {
  return { prism, submitter, visibility, payload: payloadOf(payloadObj), nonce };
}

// ---- Token Prism (amounts are decimal strings, matching Rust u128 serde) ----

export function tokenCreate(
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  name: string,
  symbol: string,
  decimals: number,
  initialSupply: bigint | number | string,
  mintable: boolean,
): Command {
  return cmd("token", submitter, visibility, nonce, {
    op: "create",
    name,
    symbol,
    decimals,
    initial_supply: String(initialSupply),
    mintable,
  });
}

export function tokenTransfer(
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  tokenIdHex: string,
  to: PartyId,
  amount: bigint | number | string,
): Command {
  return cmd("token", submitter, visibility, nonce, {
    op: "transfer",
    token_id: hashBytes(tokenIdHex),
    to,
    amount: String(amount),
  });
}

// ---- NFT Prism ----

export function nftCreateCollection(
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  name: string,
  symbol: string,
  maxSupply: number | null,
): Command {
  return cmd("nft", submitter, visibility, nonce, {
    op: "create_collection",
    name,
    symbol,
    max_supply: maxSupply,
  });
}

// ---- Storage Prism ----

export function storagePut(
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  key: string,
  data: Uint8Array | number[],
): Command {
  return cmd("storage", submitter, visibility, nonce, {
    key,
    data: Array.from(data),
  });
}

// ---- Contract Prism (PhotonVM) ----

export function contractDeploy(
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  code: Uint8Array | number[],
): Command {
  return cmd("contract", submitter, visibility, nonce, {
    op: "deploy",
    code: Array.from(code),
  });
}

export function contractCall(
  submitter: PartyId,
  visibility: Visibility,
  nonce: number,
  addressHex: string,
  args: number[],
  value: number,
  gasLimit: number,
): Command {
  return cmd("contract", submitter, visibility, nonce, {
    op: "call",
    address: hashBytes(addressHex),
    args,
    value,
    gas_limit: gasLimit,
  });
}
