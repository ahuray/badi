import { readFile } from "node:fs/promises";
import path from "node:path";
import Ajv2020 from "ajv/dist/2020.js";
import {
  assertV3LinkedRun,
  assertV3ProductCell,
  assertV3ReceiptSemantics,
  assertV3RunSemantics,
} from "./policy.mjs";

export const V3_DOCUMENT_KIND = Object.freeze({
  productCell: "product-cell",
  run: "run",
});

export const V3_SCHEMA_IDENTITIES = Object.freeze({
  productCell: "./v3/product-cell.schema.json",
  run: "../v3/run.schema.json",
});

export const V3_PRODUCT_CELL_PATH_PATTERN =
  /^capabilities\/(badi-product-cell\.[a-z0-9][a-z0-9-]{0,63}\.v3)\.json$/u;
export const V3_EVIDENCE_PATH_PATTERN =
  /^capabilities\/evidence\/(badi-(?:semantic|chromium|omarchy)-run\.[a-z0-9][a-z0-9-]{0,63}\.v3)\.json$/u;
const V3_PRODUCT_CELL_ID_PATTERN =
  /^badi-product-cell\.[a-z0-9][a-z0-9-]{0,63}\.v3$/u;
const V3_RUN_ID_PATTERN =
  /^badi-(?:semantic|chromium|omarchy)-run\.[a-z0-9][a-z0-9-]{0,63}\.v3$/u;

const schemaFiles = Object.freeze({
  [V3_DOCUMENT_KIND.productCell]: "product-cell.schema.json",
  [V3_DOCUMENT_KIND.run]: "run.schema.json",
});
const validators = new Map();

function hasV3Signal(relativePath, value) {
  return (
    V3_PRODUCT_CELL_PATH_PATTERN.test(relativePath) ||
    V3_EVIDENCE_PATH_PATTERN.test(relativePath) ||
    relativePath.endsWith(".v3.json") ||
    value?.record_version === 3 ||
    V3_PRODUCT_CELL_ID_PATTERN.test(value?.id) ||
    V3_RUN_ID_PATTERN.test(value?.id) ||
    value?.$schema === V3_SCHEMA_IDENTITIES.productCell ||
    value?.$schema === V3_SCHEMA_IDENTITIES.run
  );
}

export function classifyV3Document(relativePath, value) {
  if (!hasV3Signal(relativePath, value)) return null;

  const productCellMatch = V3_PRODUCT_CELL_PATH_PATTERN.exec(relativePath);
  const runMatch = V3_EVIDENCE_PATH_PATTERN.exec(relativePath);
  const kind = productCellMatch === null
    ? runMatch === null
      ? null
      : V3_DOCUMENT_KIND.run
    : V3_DOCUMENT_KIND.productCell;
  const identityMatch = productCellMatch ?? runMatch;
  const expectedSchema = kind === V3_DOCUMENT_KIND.productCell
    ? V3_SCHEMA_IDENTITIES.productCell
    : V3_SCHEMA_IDENTITIES.run;

  if (
    kind === null ||
    !Number.isSafeInteger(value?.record_version) ||
    value.record_version !== 3 ||
    value.$schema !== expectedSchema ||
    value.id !== identityMatch[1]
  ) {
    throw new Error(
      `${relativePath}: location, id, or schema identity differs for record_version 3`,
    );
  }
  return kind;
}

async function validatorFor(kind) {
  let validate = validators.get(kind);
  if (validate !== undefined) return validate;

  const schema = JSON.parse(
    await readFile(path.join(import.meta.dirname, schemaFiles[kind]), "utf8"),
  );
  validate = new Ajv2020({ allErrors: true, strict: true }).compile(schema);
  validators.set(kind, validate);
  return validate;
}

export async function validateV3Document(relativePath, value) {
  const kind = classifyV3Document(relativePath, value);
  if (kind === null) return null;

  const validate = await validatorFor(kind);
  if (!validate(value)) {
    throw new Error(
      `${relativePath}: ${JSON.stringify(validate.errors, null, 2)}`,
    );
  }
  if (kind === V3_DOCUMENT_KIND.productCell) {
    assertV3ReceiptSemantics(relativePath, value);
  } else {
    assertV3RunSemantics(relativePath, value);
  }
  return kind;
}

export function assertV3LinkedProductCell(file, receipt, linkedRuns) {
  for (const link of receipt.linked_evidence) {
    assertV3LinkedRun(file, receipt, link, linkedRuns.get(link.kind));
  }
  assertV3ProductCell(file, receipt, linkedRuns);
}

export function assertV3PostImplementationChanges(file, changes) {
  for (const line of changes.trimEnd().split("\n").filter(Boolean)) {
    const [statusCode, relativePath, unexpectedPath] = line.split("\t");
    if (
      statusCode !== "A" ||
      unexpectedPath !== undefined ||
      (!V3_PRODUCT_CELL_PATH_PATTERN.test(relativePath) &&
        !V3_EVIDENCE_PATH_PATTERN.test(relativePath))
    ) {
      throw new Error(
        `${file}: non-evidence change follows the implementation commit: ${line}`,
      );
    }
  }
}

export function assertV3ProductCellFileIdentity(relativePath, receipt) {
  let kind;
  try {
    kind = classifyV3Document(relativePath, receipt);
  } catch {
    throw new Error(`New V3 product-cell receipt identity is invalid: ${relativePath}`);
  }
  if (
    kind !== V3_DOCUMENT_KIND.productCell ||
    !["candidate", "live", "unsupported"].includes(receipt.status)
  ) {
    throw new Error(`New V3 product-cell receipt identity is invalid: ${relativePath}`);
  }
}

export function assertV3ProductCellAppendOnly(status, relativePath) {
  const pathMatch = V3_PRODUCT_CELL_PATH_PATTERN.exec(relativePath);
  if (
    status !== "A" &&
    (pathMatch !== null || relativePath.endsWith(".v3.json"))
  ) {
    throw new Error(
      `V3 product-cell receipts are append-only: ${status} ${relativePath}`,
    );
  }
  return pathMatch;
}

export function looksLikeV3ProductCell(relativePath, receipt) {
  return (
    V3_PRODUCT_CELL_PATH_PATTERN.test(relativePath) ||
    relativePath.endsWith(".v3.json") ||
    receipt?.record_version === 3 ||
    receipt?.$schema === V3_SCHEMA_IDENTITIES.productCell ||
    receipt?.$schema === V3_SCHEMA_IDENTITIES.run
  );
}
