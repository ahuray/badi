import type {
  CommitAuthorization,
  CommitAuthorizationRequest,
  CommitResultNotice,
  SuggestionAddress,
  SuggestionRequest,
  SuggestionResponse,
  SuggestionTransport,
} from "../shared/model";
import {
  parseRuntimeBootstrapReply,
  parseRuntimeCommitAuthorization,
  parseRuntimeSuggestionReply,
  type RuntimeCommand,
  type RuntimeReply,
} from "../shared/runtime-messages";

export interface RuntimeMessenger {
  sendMessage(message: unknown): Promise<unknown>;
}

export class RuntimeSuggestionTransport implements SuggestionTransport {
  readonly #messenger: RuntimeMessenger;
  readonly #sessionClosures = new Map<string, Promise<void>>();

  constructor(messenger: RuntimeMessenger = chrome.runtime) {
    this.#messenger = messenger;
  }

  async bootstrap(sessionId: string): Promise<boolean> {
    const reply = await this.#messenger.sendMessage({
      kind: "badi.bootstrap.v1",
      sessionId,
    } satisfies RuntimeCommand);
    return parseRuntimeBootstrapReply(reply);
  }

  async requestSuggestion(request: SuggestionRequest): Promise<SuggestionResponse> {
    await this.#waitForSessionClose(request.sessionId);
    const reply = await this.#messenger.sendMessage({
      kind: "badi.suggest.v1",
      request,
    } satisfies RuntimeCommand);
    return parseRuntimeSuggestionReply(reply);
  }

  async cancelSuggestion(request: SuggestionRequest): Promise<void> {
    await this.#notify({ kind: "badi.cancel.v1", request });
  }

  closeSession(sessionId: string): Promise<void> {
    const previous = this.#sessionClosures.get(sessionId);
    const close =
      previous === undefined
        ? this.#notify({ kind: "badi.session.close.v1", sessionId })
        : previous
            .catch(() => undefined)
            .then(() => this.#notify({ kind: "badi.session.close.v1", sessionId }));
    this.#sessionClosures.set(sessionId, close);
    return close.finally(() => {
      if (this.#sessionClosures.get(sessionId) === close) {
        this.#sessionClosures.delete(sessionId);
      }
    });
  }

  async dismissSuggestion(address: SuggestionAddress): Promise<void> {
    await this.#notify({ kind: "badi.dismiss.v1", address });
  }

  async authorizeCommit(
    request: CommitAuthorizationRequest,
  ): Promise<CommitAuthorization> {
    const reply = await this.#messenger.sendMessage({
      kind: "badi.commit.authorize.v1",
      request,
    } satisfies RuntimeCommand);
    return parseRuntimeCommitAuthorization(reply);
  }

  async reportCommit(notice: CommitResultNotice): Promise<void> {
    await this.#notify({ kind: "badi.commit.result.v1", notice });
  }

  async #notify(command: RuntimeCommand): Promise<void> {
    const value = await this.#messenger.sendMessage(command);
    if (
      typeof value !== "object" ||
      value === null ||
      !("ok" in value) ||
      (value as RuntimeReply).ok !== true
    ) {
      throw new Error("Extension service worker rejected a Badi message");
    }
  }

  async #waitForSessionClose(sessionId: string): Promise<void> {
    while (true) {
      const close = this.#sessionClosures.get(sessionId);
      if (close === undefined) return;
      await close.catch(() => undefined);
    }
  }
}
