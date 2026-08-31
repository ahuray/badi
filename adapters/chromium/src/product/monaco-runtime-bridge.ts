import {
  parseApplyReply,
  parseSnapshotReply,
  type ProductBridgeCommand,
} from "./messages";
import type { MonacoSnapshot, MonacoSnapshotGuard } from "./monaco-main-world";

export interface MonacoBridge {
  snapshot(sessionId: string): Promise<MonacoSnapshot | null>;
  apply(
    sessionId: string,
    expected: MonacoSnapshotGuard,
    authorization: CommitAuthorization,
  ): Promise<boolean>;
}

export interface ProductRuntimeMessenger {
  sendMessage(message: unknown): Promise<unknown>;
}

export class RuntimeMonacoBridge implements MonacoBridge {
  readonly #messenger: ProductRuntimeMessenger;

  constructor(messenger: ProductRuntimeMessenger = chrome.runtime) {
    this.#messenger = messenger;
  }

  async snapshot(sessionId: string): Promise<MonacoSnapshot | null> {
    const reply = await this.#messenger.sendMessage({
      kind: "badi.product.monaco.snapshot.v1",
      sessionId,
    } satisfies ProductBridgeCommand);
    return parseSnapshotReply(reply);
  }

  async apply(
    sessionId: string,
    expected: MonacoSnapshotGuard,
    authorization: CommitAuthorization,
  ): Promise<boolean> {
    const reply = await this.#messenger.sendMessage({
      kind: "badi.product.monaco.apply.v1",
      sessionId,
      expected,
      authorization,
    } satisfies ProductBridgeCommand);
    return parseApplyReply(reply);
  }
}
import type { CommitAuthorization } from "../shared/model";
