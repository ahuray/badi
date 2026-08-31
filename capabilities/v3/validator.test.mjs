import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import {
  V3_DOCUMENT_KIND,
  classifyV3Document,
  validateV3Document,
} from "./validator.mjs";

const receiptPath = "capabilities/badi-product-cell.fixture.v3.json";
const runPath = "capabilities/evidence/badi-semantic-run.fixture.v3.json";
const receiptIdentity = Object.freeze({
  $schema: "./v3/product-cell.schema.json",
  record_version: 3,
  id: "badi-product-cell.fixture.v3",
});
const runIdentity = Object.freeze({
  $schema: "../v3/run.schema.json",
  record_version: 3,
  id: "badi-semantic-run.fixture.v3",
});

test("V3 classification binds location, version, schema, and id", () => {
  assert.equal(
    classifyV3Document(receiptPath, receiptIdentity),
    V3_DOCUMENT_KIND.productCell,
  );
  assert.equal(classifyV3Document(runPath, runIdentity), V3_DOCUMENT_KIND.run);
  assert.equal(
    classifyV3Document("capabilities/legacy.json", {
      $schema: "./v2/schema.json",
      record_version: 2,
      id: "legacy",
    }),
    null,
  );

  for (const [relativePath, value] of [
    [receiptPath, { ...receiptIdentity, record_version: 2 }],
    [receiptPath, { ...receiptIdentity, $schema: runIdentity.$schema }],
    [receiptPath, { ...receiptIdentity, id: "badi-product-cell.other.v3" }],
    [runPath, { ...runIdentity, $schema: receiptIdentity.$schema }],
    ["capabilities/wrong.v3.json", receiptIdentity],
    ["capabilities/legacy.json", { ...receiptIdentity }],
    [
      "capabilities/legacy.json",
      {
        $schema: "./v2/schema.json",
        record_version: 2,
        id: receiptIdentity.id,
      },
    ],
  ]) {
    assert.throws(
      () => classifyV3Document(relativePath, value),
      /location, id, or schema identity differs for record_version 3/u,
    );
  }
});

test("V3 validation loads the fixed schema selected by protected classification", async () => {
  await assert.rejects(
    validateV3Document(receiptPath, receiptIdentity),
    /must have required property/u,
  );
  await assert.rejects(
    validateV3Document(runPath, runIdentity),
    /must have required property/u,
  );
});

test("the generic checker delegates V3 selection and dispatch", async () => {
  const checker = await readFile(
    path.join(import.meta.dirname, "../../scripts/check-capabilities.mjs"),
    "utf8",
  );
  assert.match(checker, /from "\.\.\/capabilities\/v3\/validator\.mjs"/u);
  assert.match(checker, /validateV3Document/u);
  assert.doesNotMatch(checker, /capabilities\/v3\/policy\.mjs/u);
  assert.doesNotMatch(checker, /\.\/v3\/product-cell\.schema\.json/u);
  assert.doesNotMatch(checker, /\.\.\/v3\/run\.schema\.json/u);
  assert.doesNotMatch(checker, /record_version\s*[!=]==?\s*3/u);
});
