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
    // Project the snapshot onto the strict wire guard. Callers may hold the
    // richer MonacoSnapshot shape, whose geometry is not part of an edit
    // authorization and must not cross this exact-key boundary.
    const guard: MonacoSnapshotGuard = {
      modelUri: expected.modelUri,
      languageId: expected.languageId,
      versionId: expected.versionId,
      valueLength: expected.valueLength,
      offset: expected.offset,
      lineNumber: expected.lineNumber,
      column: expected.column,
      before: expected.before,
      after: expected.after,
    };
    const reply = await this.#messenger.sendMessage({
      kind: "badi.product.monaco.apply.v1",
      sessionId,
      expected: guard,
      authorization,
    } satisfies ProductBridgeCommand);
    return parseApplyReply(reply);
  }
}
import type { CommitAuthorization } from "../shared/model";
