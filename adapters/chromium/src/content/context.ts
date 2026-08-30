import type {
  EditableField,
  FieldDescriptor,
  SelectionSnapshot,
  SuggestionContext,
} from "../shared/model";
import { hasStableFieldIdentity } from "./field-policy";

const BEFORE_LIMIT = 512;
const AFTER_LIMIT = 128;

function firstScalars(value: string, limit: number): string {
  return Array.from(value).slice(0, limit).join("");
}

function lastScalars(value: string, limit: number): string {
  return Array.from(value).slice(-limit).join("");
}

function hash(value: string): string {
  const lanes = [0x811c9dc5, 0x9e3779b9, 0x85ebca6b, 0xc2b2ae35];
  for (const character of value) {
    const scalar = character.codePointAt(0) ?? 0;
    for (let index = 0; index < lanes.length; index += 1) {
      const lane = lanes[index] ?? 0;
      const mixed = lane ^ (scalar + Math.imul(index + 1, 0x9e3779b9));
      lanes[index] = Math.imul(mixed, 0x01000193 + index * 2);
    }
  }
  return lanes
    .map((lane) => (lane >>> 0).toString(16).padStart(8, "0"))
    .join("");
}

function stableIdentity(field: EditableField): string {
  const tag = field instanceof HTMLTextAreaElement ? "textarea" : `input:${field.type}`;
  return [
    tag,
    field.getAttribute("data-badi-field") ?? "",
    field.id,
    field.getAttribute("name") ?? "",
  ].join("|");
}

export interface ContextCaptureInput {
  readonly field: EditableField;
  readonly purpose: FieldDescriptor["purpose"];
  readonly selection: SelectionSnapshot;
  readonly composing: boolean;
  readonly activation: "always" | "manual";
  readonly explicit: boolean;
  readonly fingerprintSalt: string;
}

export function captureContext(input: ContextCaptureInput): SuggestionContext {
  const { field, purpose, selection } = input;
  if (!hasStableFieldIdentity(field)) {
    throw new Error("Refusing to capture context for a field without stable identity");
  }
  const value = field.value;
  const beforeSlice = value.slice(0, selection.start);
  const afterSlice = value.slice(selection.end);
  if (hasUnpairedSurrogate(beforeSlice) || hasUnpairedSurrogate(afterSlice)) {
    throw new Error("Refusing to capture ill-formed UTF-16 context");
  }
  const before = lastScalars(beforeSlice, BEFORE_LIMIT);
  const after = firstScalars(afterSlice, AFTER_LIMIT);
  const identity = stableIdentity(field);
  const fingerprint = hash(
    [input.fingerprintSalt, identity, before, after, selection.start, selection.end].join(
      "\u001f",
    ),
  );

  return {
    fingerprint,
    before,
    after,
    selection,
    field: {
      purpose,
      editable: true,
      multiline: field instanceof HTMLTextAreaElement,
      composing: input.composing,
      sensitive: false,
      identityKnown: true,
    },
    activation: input.activation,
    explicit: input.explicit,
  };
}

export function captureContextOrNull(
  input: ContextCaptureInput,
): SuggestionContext | null {
  try {
    return captureContext(input);
  } catch {
    return null;
  }
}

function hasUnpairedSurrogate(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) {
        return true;
      }
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true;
    }
  }
  return false;
}

export function sanitizeSuggestion(value: string): string | null {
  if (
    value.length === 0 ||
    hasUnpairedSurrogate(value) ||
    /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/u.test(value) ||
    /\p{Cf}/u.test(value)
  ) {
    return null;
  }
  const scalars = Array.from(value);
  if (scalars.length > 64 || (value.match(/\S+/gu) ?? []).length > 8) {
    return null;
  }
  return value;
}

export function nextSuggestionWord(value: string): string {
  const word = value.match(/^\s*[\p{L}\p{N}\p{M}_]+/u);
  if (word?.[0]) {
    return word[0];
  }
  const symbol = value.match(/^\s*[^\s\p{L}\p{N}\p{M}_]/u);
  if (symbol?.[0]) {
    return symbol[0];
  }
  return value;
}
