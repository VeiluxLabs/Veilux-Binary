import {
  RPC_METHODS,
  type BlockView,
  type EstimateResult,
  type NodeInfo,
  type SignedCommand,
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
}
