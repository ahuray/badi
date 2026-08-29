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

  constructor(messenger: RuntimeMessenger = chrome.runtime) {
    this.#messenger = messenger;
  }

  async requestSuggestion(request: SuggestionRequest): Promise<SuggestionResponse> {
    const reply = await this.#messenger.sendMessage({
      kind: "omatype.suggest.v1",
      request,
    } satisfies RuntimeCommand);
    return parseRuntimeSuggestionReply(reply);
  }

  async cancelSuggestion(request: SuggestionRequest): Promise<void> {
    await this.#notify({ kind: "omatype.cancel.v1", request });
  }

  async dismissSuggestion(address: SuggestionAddress): Promise<void> {
    await this.#notify({ kind: "omatype.dismiss.v1", address });
  }

  async authorizeCommit(
    request: CommitAuthorizationRequest,
  ): Promise<CommitAuthorization> {
    const reply = await this.#messenger.sendMessage({
      kind: "omatype.commit.authorize.v1",
      request,
    } satisfies RuntimeCommand);
    return parseRuntimeCommitAuthorization(reply);
  }

  async reportCommit(notice: CommitResultNotice): Promise<void> {
    await this.#notify({ kind: "omatype.commit.result.v1", notice });
  }

  async #notify(command: RuntimeCommand): Promise<void> {
    const value = await this.#messenger.sendMessage(command);
    if (
      typeof value !== "object" ||
      value === null ||
      !("ok" in value) ||
      (value as RuntimeReply).ok !== true
    ) {
      throw new Error("Extension service worker rejected an Omatype message");
    }
  }
}
