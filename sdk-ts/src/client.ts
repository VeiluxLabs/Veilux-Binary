import {
  RPC_METHODS,
  type BlockView,
  type ChainStats,
  type CommandLocation,
  type EstimateResult,
  type EventView,
  type NodeInfo,
  type SignedCommand,
  type StatePrefixResult,
  type StateResult,
  type SubmitResult,
} from "./types.js";

interface RpcResponse<T> {
  jsonrpc: string;
  result?: T;
  error?: { code: number; message: string };
  id: unknown;
}

export class RpcClientError extends Error {
  constructor(public code: number, message: string) {
    super(`rpc error ${code}: ${message}`);
    this.name = "RpcClientError";
  }
}

/** A JSON-RPC client for a VEILUX node. Uses the global `fetch`. */
export class Client {
  private id = 1;

  constructor(private readonly endpoint: string) {}

  private async call<T>(method: string, params: unknown): Promise<T> {
    const body = JSON.stringify({
      jsonrpc: "2.0",
      method,
      params,
      id: this.id++,
    });
    const resp = await fetch(this.endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body,
    });
    const json = (await resp.json()) as RpcResponse<T>;
    if (json.error) {
      throw new RpcClientError(json.error.code, json.error.message);
    }
    if (json.result === undefined) {
      throw new Error("missing result in RPC response");
    }
    return json.result;
  }

  nodeInfo(): Promise<NodeInfo> {
    return this.call(RPC_METHODS.nodeInfo, {});
  }

  blockNumber(): Promise<number> {
    return this.call(RPC_METHODS.blockNumber, {});
  }

  blockByNumber(height: number): Promise<BlockView> {
    return this.call(RPC_METHODS.getBlockByNumber, { height });
  }

  getState(key: string): Promise<StateResult> {
    return this.call(RPC_METHODS.getState, { key });
  }

  estimate(command: SignedCommand): Promise<EstimateResult> {
    return this.call(RPC_METHODS.estimate, { command });
  }

  submit(command: SignedCommand): Promise<SubmitResult> {
    return this.call(RPC_METHODS.submit, { command });
  }

  /**
   * Read a token balance as a bigint (0 if the account has none).
   * `tokenIdHex` is the `0x`-prefixed token id (see `ids.tokenId`).
   */
  async tokenBalance(tokenIdHex: string, party: string): Promise<bigint> {
    const r = await this.getState(`token/bal/${tokenIdHex}/${party}`);
    if (!r.found) return 0n;
    const clean = r.value_hex.startsWith("0x") ? r.value_hex.slice(2) : r.value_hex;
    const bytes = new Uint8Array(clean.length / 2);
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
    }
    const text = new TextDecoder().decode(bytes);
    return text ? BigInt(text) : 0n;
  }

  /**
   * Poll until the chain reaches at least `target` height (or times out).
   * Useful after submitting to a non-instant-mining node.
   */
  async waitForHeight(target: number, opts?: { timeoutMs?: number; intervalMs?: number }): Promise<number> {
    const timeoutMs = opts?.timeoutMs ?? 30_000;
    const intervalMs = opts?.intervalMs ?? 500;
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const h = await this.blockNumber();
      if (h >= target) return h;
      if (Date.now() > deadline) {
        throw new Error(`timed out waiting for height ${target} (current ${h})`);
      }
      await new Promise((r) => setTimeout(r, intervalMs));
    }
  }

  // ---- Explorer queries ----

  /** Chain-wide statistics for dashboards. */
  explorerStats(): Promise<ChainStats> {
    return this.call(RPC_METHODS.explorerStats, {});
  }

  /** The most recent blocks, newest first. */
  recentBlocks(limit = 20): Promise<BlockView[]> {
    return this.call(RPC_METHODS.explorerRecentBlocks, { limit });
  }

  /** Look up a block by its hash. */
  blockByHash(hash: string): Promise<BlockView> {
    return this.call(RPC_METHODS.explorerBlockByHash, { hash });
  }

  /** Locate a command by id and return its block + produced events. */
  searchCommand(commandId: string): Promise<CommandLocation> {
    return this.call(RPC_METHODS.explorerSearchCommand, { command_id: commandId });
  }

  /** Recent events emitted by a given Prism (e.g. "token", "bridge"). */
  listByPrism(prism: string, limit = 50): Promise<EventView[]> {
    return this.call(RPC_METHODS.explorerListByPrism, { prism, limit });
  }

  /** List state entries under a key prefix (e.g. "token/meta/"). */
  statePrefix(prefix: string, limit = 100): Promise<StatePrefixResult> {
    return this.call(RPC_METHODS.explorerStatePrefix, { prefix, limit });
  }
}
