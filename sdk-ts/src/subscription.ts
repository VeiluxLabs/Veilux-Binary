import type { BlockNotification } from "./types.js";

export interface Subscription {
  /** Close the underlying WebSocket and stop receiving notifications. */
  close(): void;
}

export interface SubscribeHandlers {
  /** Called for every committed block. */
  onBlock?: (block: BlockNotification) => void;
  /** Called once the subscription is established. */
  onOpen?: () => void;
  /** Called on socket error. */
  onError?: (err: unknown) => void;
  /** Called when the socket closes. */
  onClose?: () => void;
}

/**
 * Subscribe to real-time block notifications over WebSocket.
 *
 * Works in the browser and in Node.js 20+ (both expose a global `WebSocket`).
 *
 * ```ts
 * const sub = subscribeBlocks("ws://127.0.0.1:8646", {
 *   onBlock: (b) => console.log("new block", b.height, b.hash),
 * });
 * // later: sub.close();
 * ```
 */
export function subscribeBlocks(wsUrl: string, handlers: SubscribeHandlers): Subscription {
  const WS: typeof WebSocket | undefined =
    typeof WebSocket !== "undefined" ? WebSocket : (globalThis as any).WebSocket;
  if (!WS) {
    throw new Error("No global WebSocket available. On older Node, pass a polyfill via globalThis.WebSocket.");
  }

  const ws = new WS(wsUrl);

  ws.onopen = () => handlers.onOpen?.();
  ws.onerror = (e: unknown) => handlers.onError?.(e);
  ws.onclose = () => handlers.onClose?.();
  ws.onmessage = (ev: MessageEvent) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(typeof ev.data === "string" ? ev.data : String(ev.data));
    } catch {
      return;
    }
    const msg = parsed as { type?: string };
    if (msg.type === "block" && handlers.onBlock) {
      handlers.onBlock(msg as BlockNotification);
    }
  };

  return {
    close() {
      ws.close();
    },
  };
}
