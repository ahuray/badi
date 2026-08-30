export type EditableField = HTMLInputElement | HTMLTextAreaElement;

export type SelectionDirection = "forward" | "backward" | "none";

export interface SelectionSnapshot {
  readonly start: number;
  readonly end: number;
  readonly direction: SelectionDirection;
}

export interface FieldDescriptor {
  readonly purpose:
    | "normal"
    | "password"
    | "pin"
    | "otp"
    | "payment_secret"
    | "terminal"
    | "email"
    | "url"
    | "search"
    | "unknown";
  readonly editable: true;
  readonly multiline: boolean;
  readonly composing: boolean;
  readonly sensitive: false;
  readonly identityKnown: boolean;
}

export interface SuggestionContext {
  readonly fingerprint: string;
  readonly before: string;
  readonly after: string;
  readonly selection: SelectionSnapshot;
  readonly field: FieldDescriptor;
  readonly activation: "always" | "manual";
  readonly explicit: boolean;
}

export interface SuggestionRequest {
  readonly requestId: string;
  readonly sessionId: string;
  readonly origin: string;
  readonly focusEpoch: number;
  readonly revision: number;
  readonly monotonicMs: number;
  readonly context: SuggestionContext;
}

export interface SuggestionResponse {
  readonly requestId: string;
  readonly sessionId: string;
  readonly focusEpoch: number;
  readonly revision: number;
  readonly fingerprint: string;
  readonly suggestion: string | null;
  readonly suggestionId: string | null;
  readonly acceptWord: string | null;
  readonly ttlMs: number | null;
}

export interface SuggestionAddress {
  readonly requestId: string;
  readonly sessionId: string;
  readonly focusEpoch: number;
  readonly revision: number;
  readonly monotonicMs: number;
  readonly fingerprint: string;
  readonly suggestionId: string;
}

export interface SuggestionClearEvent {
  readonly requestId: string | null;
  readonly sessionId: string;
  readonly focusEpoch: number;
  readonly revision: number;
  readonly monotonicMs: number;
  readonly fingerprint: string;
  readonly suggestionId: string | null;
  readonly reason: string;
}

export interface CommitAuthorizationRequest extends SuggestionAddress {
  readonly expectedText: string;
  readonly acceptance: "word" | "all";
}

export interface CommitAuthorization extends SuggestionAddress {
  readonly text: string;
  readonly acceptance: "word" | "all";
}

export interface CommitResultNotice extends SuggestionAddress {
  readonly status: "applied" | "dispatched-unverified" | "stale" | "blocked" | "failed";
  readonly newRevision?: number;
  readonly newFingerprint?: string;
}

export interface SuggestionTransport {
  requestSuggestion(request: SuggestionRequest): Promise<SuggestionResponse>;
  cancelSuggestion(request: SuggestionRequest): void | Promise<void>;
  closeSession?(sessionId: string): void | Promise<void>;
  dismissSuggestion?(address: SuggestionAddress): void | Promise<void>;
  authorizeCommit(request: CommitAuthorizationRequest): Promise<CommitAuthorization>;
  reportCommit(notice: CommitResultNotice): void | Promise<void>;
  dispose?(): void;
}

export interface SuggestionView {
  readonly visible: boolean;
  show(field: EditableField, text: string): void;
  hide(): void;
  dispose(): void;
}

export function readSelection(field: EditableField): SelectionSnapshot | null {
  const { selectionStart, selectionEnd } = field;
  if (selectionStart === null || selectionEnd === null) {
    return null;
  }

  const direction = field.selectionDirection;
  return {
    start: selectionStart,
    end: selectionEnd,
    direction:
      direction === "forward" || direction === "backward" ? direction : "none",
  };
}

export function selectionsEqual(
  left: SelectionSnapshot,
  right: SelectionSnapshot,
): boolean {
  return (
    left.start === right.start &&
    left.end === right.end &&
    left.direction === right.direction
  );
}
