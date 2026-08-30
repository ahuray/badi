import type { EditableField } from "../shared/model";

type AllowedPurpose = "normal" | "unknown";

export type DenialReason =
  | "not-a-supported-field"
  | "password"
  | "hidden"
  | "sensitive-autocomplete"
  | "not-editable"
  | "not-visible"
  | "not-top-level-light-dom"
  | "identity-unknown"
  | "page-opt-out";

export type FieldPolicyDecision =
  | {
      readonly allowed: true;
      readonly field: EditableField;
      readonly purpose: AllowedPurpose;
    }
  | {
      readonly allowed: false;
      readonly reason: DenialReason;
    };

const SUPPORTED_INPUT_TYPES = new Set(["text"]);

// The WHATWG autocomplete purposes that can directly identify a person,
// credential, address, phone number, birthday, or payment instrument. This is a
// local hard boundary: these fields never reach context capture or transport.
const SENSITIVE_AUTOCOMPLETE_TOKENS = new Set([
  "name",
  "honorific-prefix",
  "given-name",
  "additional-name",
  "family-name",
  "honorific-suffix",
  "nickname",
  "username",
  "new-password",
  "current-password",
  "one-time-code",
  "organization-title",
  "organization",
  "street-address",
  "address-line1",
  "address-line2",
  "address-line3",
  "address-level4",
  "address-level3",
  "address-level2",
  "address-level1",
  "country",
  "country-name",
  "postal-code",
  "cc-name",
  "cc-given-name",
  "cc-additional-name",
  "cc-family-name",
  "cc-number",
  "cc-exp",
  "cc-exp-month",
  "cc-exp-year",
  "cc-csc",
  "cc-type",
  "transaction-currency",
  "transaction-amount",
  "language",
  "bday",
  "bday-day",
  "bday-month",
  "bday-year",
  "sex",
  "url",
  "photo",
  "tel",
  "tel-country-code",
  "tel-national",
  "tel-area-code",
  "tel-local",
  "tel-local-prefix",
  "tel-local-suffix",
  "tel-extension",
  "email",
  "impp",
  "webauthn",
]);

function autocompleteTokens(field: EditableField): readonly string[] {
  return (field.getAttribute("autocomplete") ?? "")
    .trim()
    .toLowerCase()
    .split(/\s+/u)
    .filter(Boolean);
}

function hasPageOptOut(field: EditableField): boolean {
  return field.closest("[data-badi='off']") !== null;
}

function isTopLevelLightDom(field: EditableField): boolean {
  if (field.getRootNode() !== field.ownerDocument) {
    return false;
  }
  const view = field.ownerDocument.defaultView;
  try {
    return view !== null && view === view.top;
  } catch {
    return false;
  }
}

function isVisible(field: EditableField): boolean {
  if (
    field.hidden ||
    field.closest("[hidden], [inert], [aria-hidden='true']") !== null
  ) {
    return false;
  }
  if (typeof field.checkVisibility === "function") {
    if (!field.checkVisibility({
      checkOpacity: true,
      checkVisibilityCSS: true,
    })) {
      return false;
    }
    const rect = field.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }
  const rect = field.getBoundingClientRect();
  return field.getClientRects().length > 0 && rect.width > 0 && rect.height > 0;
}

function exactlyOneWithAttribute(
  root: ParentNode,
  selector: string,
  attribute: string,
  value: string,
  field: EditableField,
): boolean {
  const matches = Array.from(root.querySelectorAll(selector)).filter(
    (candidate) => candidate.getAttribute(attribute) === value,
  );
  return matches.length === 1 && matches[0] === field;
}

export function hasStableFieldIdentity(field: EditableField): boolean {
  if (!field.isConnected) {
    return false;
  }
  const document = field.ownerDocument;
  const explicit = field.getAttribute("data-badi-field")?.trim() ?? "";
  if (
    explicit.length > 0 &&
    exactlyOneWithAttribute(
      document,
      "[data-badi-field]",
      "data-badi-field",
      explicit,
      field,
    )
  ) {
    return true;
  }
  if (
    field.id.trim().length > 0 &&
    exactlyOneWithAttribute(document, "[id]", "id", field.id, field)
  ) {
    return true;
  }
  const name = field.getAttribute("name")?.trim() ?? "";
  const scope: ParentNode = field.form ?? document;
  return (
    name.length > 0 &&
    exactlyOneWithAttribute(scope, "input[name],textarea[name]", "name", name, field)
  );
}

export function evaluateField(field: Element): FieldPolicyDecision {
  if (!(field instanceof HTMLInputElement) && !(field instanceof HTMLTextAreaElement)) {
    return { allowed: false, reason: "not-a-supported-field" };
  }

  if (field instanceof HTMLInputElement) {
    const inputType = field.type.toLowerCase();
    if (inputType === "password") {
      return { allowed: false, reason: "password" };
    }
    if (inputType === "hidden") {
      return { allowed: false, reason: "hidden" };
    }
    if (!SUPPORTED_INPUT_TYPES.has(inputType)) {
      return { allowed: false, reason: "not-a-supported-field" };
    }
  }

  if (field.disabled || field.readOnly) {
    return { allowed: false, reason: "not-editable" };
  }
  if (!isTopLevelLightDom(field)) {
    return { allowed: false, reason: "not-top-level-light-dom" };
  }
  if (!isVisible(field)) {
    return { allowed: false, reason: "not-visible" };
  }
  if (hasPageOptOut(field)) {
    return { allowed: false, reason: "page-opt-out" };
  }

  const tokens = autocompleteTokens(field);
  if (tokens.some((token) => SENSITIVE_AUTOCOMPLETE_TOKENS.has(token))) {
    return { allowed: false, reason: "sensitive-autocomplete" };
  }
  if (!hasStableFieldIdentity(field)) {
    return { allowed: false, reason: "identity-unknown" };
  }

  const declaredPurpose = tokens.find(
    (token) =>
      token !== "on" &&
      token !== "off" &&
      token !== "shipping" &&
      token !== "billing" &&
      !token.startsWith("section-"),
  );

  return {
    allowed: true,
    field,
    purpose: declaredPurpose === undefined ? "normal" : "unknown",
  };
}
