import type {
  EditableField,
  FieldDescriptor,
  SelectionSnapshot,
  SuggestionContext,
} from "../shared/model";
import { hasStableFieldIdentity } from "./field-policy";

const BEFORE_LIMIT = 512;
const AFTER_LIMIT = 128;
const SUGGESTION_SCALAR_LIMIT = 64;
const SUGGESTION_WORD_LIMIT = 8;
const WORD_SEGMENTER = new Intl.Segmenter("und", { granularity: "word" });
const GRAPHEME_SEGMENTER = new Intl.Segmenter("und", { granularity: "grapheme" });

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
  if (value.length === 0 || hasUnpairedSurrogate(value)) {
    return null;
  }

  for (const character of value) {
    if (isForbiddenOutputScalar(character)) return null;
  }

  let output = "";
  let scalarCount = 0;
  let wordCount = 0;
  for (const segment of WORD_SEGMENTER.segment(value)) {
    const boundaryWords = segment.isWordLike ? 1 : 0;
    if (boundaryWords > 0 && wordCount + boundaryWords > SUGGESTION_WORD_LIMIT) {
      break;
    }
    for (const character of segment.segment) {
      if (scalarCount === SUGGESTION_SCALAR_LIMIT) break;
      output += character;
      scalarCount += 1;
    }
    wordCount += boundaryWords;
    if (scalarCount === SUGGESTION_SCALAR_LIMIT) break;
  }

  output = output.replace(/\p{White_Space}+$/u, "");
  return output.length === 0 ? null : output;
}

export function nextSuggestionWord(value: string): string {
  const leading = value.match(/^\p{White_Space}*/u)?.[0] ?? "";
  const tail = value.slice(leading.length);
  const word = WORD_SEGMENTER.segment(tail)[Symbol.iterator]().next().value;
  if (word !== undefined && word.index === 0 && word.isWordLike) {
    return leading + word.segment;
  }
  const grapheme = GRAPHEME_SEGMENTER.segment(tail)[Symbol.iterator]().next().value;
  return leading + (grapheme?.segment ?? "");
}

function isForbiddenOutputScalar(character: string): boolean {
  const scalar = character.codePointAt(0) ?? 0;
  return (
    /\p{Cc}/u.test(character) ||
    scalar === 0x00ad ||
    (scalar >= 0x0600 && scalar <= 0x0605) ||
    scalar === 0x061c ||
    scalar === 0x06dd ||
    scalar === 0x070f ||
    (scalar >= 0x0890 && scalar <= 0x0891) ||
    scalar === 0x08e2 ||
    scalar === 0x180e ||
    (scalar >= 0x200b && scalar <= 0x200f) ||
    (scalar >= 0x2028 && scalar <= 0x202e) ||
    (scalar >= 0x2060 && scalar <= 0x2064) ||
    (scalar >= 0x2066 && scalar <= 0x206f) ||
    scalar === 0xfeff ||
    (scalar >= 0xfff9 && scalar <= 0xfffb) ||
    scalar === 0x110bd ||
    scalar === 0x110cd ||
    (scalar >= 0x13430 && scalar <= 0x1343f) ||
    (scalar >= 0x1bca0 && scalar <= 0x1bca3) ||
    (scalar >= 0x1d173 && scalar <= 0x1d17a) ||
    scalar === 0xe0001 ||
    (scalar >= 0xe0020 && scalar <= 0xe007f)
  );
}
