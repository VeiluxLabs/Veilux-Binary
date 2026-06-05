import { hashCommit, toHex, u64le } from "./encoding.js";
import type { PartyId } from "./types.js";

const te = new TextEncoder();

/**
 * Derive a token id exactly like the Token Prism:
 * `keccak/blake3 commit("token/id", [submitter, symbol, name])`.
 */
export function tokenId(submitter: PartyId, symbol: string, name: string): string {
  return toHex(hashCommit("token/id", [te.encode(submitter), te.encode(symbol), te.encode(name)]));
}

/**
 * Derive an NFT collection id like the NFT Prism:
 * `commit("nft/coll-id", [submitter, symbol, name])`.
 */
export function collectionId(submitter: PartyId, symbol: string, name: string): string {
  return toHex(
    hashCommit("nft/coll-id", [te.encode(submitter), te.encode(symbol), te.encode(name)]),
  );
}

/**
 * Derive a deployed contract address like the Contract Prism:
 * `commit("contract/address", [submitter, nonce_le, code])`.
 */
export function contractAddress(
  submitter: PartyId,
  nonce: number,
  code: Uint8Array | number[],
): string {
  return toHex(
    hashCommit("contract/address", [
      te.encode(submitter),
      u64le(nonce),
      Uint8Array.from(code),
    ]),
  );
}

/** State key helpers (matching each Prism's namespacing). */
export const stateKeys = {
  tokenMeta: (id: string) => `token/meta/${id}`,
  tokenBalance: (id: string, party: PartyId) => `token/bal/${id}/${party}`,
  tokenAllowance: (id: string, owner: PartyId, spender: PartyId) =>
    `token/allow/${id}/${owner}/${spender}`,
  nftCollection: (id: string) => `nft/coll/${id}`,
  nftOwner: (id: string, index: number) => `nft/owner/${id}/${index}`,
  contractCode: (addr: string) => `contract/code/${addr}`,
};

/** Decode a hex state value into a UTF-8 string (e.g. token balances). */
export function decodeStringValue(valueHex: string): string {
  const clean = valueHex.startsWith("0x") ? valueHex.slice(2) : valueHex;
  const bytes = new Uint8Array(clean.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  }
  return new TextDecoder().decode(bytes);
}
