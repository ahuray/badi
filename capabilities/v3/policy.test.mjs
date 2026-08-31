import assert from "node:assert/strict";
import test from "node:test";
import {
  V3_GATE_CONTRACTS,
  assertV3GateEvidence,
  assertV3LinkedRun,
  assertV3ReceiptSemantics,
  assertV3RunSemantics,
  computeV3QualificationSha256,
} from "./policy.mjs";

const timestamp = "2026-08-31T20:00:00Z";
const digest = "a".repeat(64);
const repository = Object.freeze({
  commit: "b".repeat(40),
  working_tree_dirty: false,
});
const hardware = Object.freeze({ cell_id: "fixture-cell" });

function validRun() {
  return {
    id: "badi-semantic-run.test.v3",
    kind: "semantic",
    status: "pass",
    recorded_at: timestamp,
    repository,
    hardware,
    qualification_sha256: digest,
    environment: { components: [{ name: "llama.cpp" }] },
    isolation: { temporary_tree_removed: true, processes_remaining: 0 },
    checks: [
      {
        id: "semantic.cases-100",
        status: "pass",
        trials: 100,
        passed: 100,
      },
    ],
    measurements: [
      {
        name: "semantic.ttft-p95-ms",
        unit: "ms",
        samples: 1_000,
        warmups: 50,
        statistic: "nearest-rank-p95",
        observations: Array(1_000).fill(200),
        value: 200,
        threshold: 250,
        threshold_operator: "at-most",
        status: "pass",
      },
    ],
    artifacts: [
      {
        id: "broker",
        locator: "repo:broker/src/lib.rs",
        repository_path: "broker/src/lib.rs",
        bytes: 1,
      },
    ],
    manual_attestations: [],
  };
}

function validReceipt() {
  const evidence = ["semantic", "chromium", "omarchy"].map((kind) => ({
    kind,
    id: `badi-${kind}-run.test.v3`,
    path: `capabilities/evidence/badi-${kind}-run.test.v3.json`,
  }));
  const roots = {
    broker: "broker/",
    adapter: "adapters/chromium/",
    evaluator: "evaluation/",
    plugin: "ui/omarchy-plugin/",
    policy: "capabilities/v3/",
    corpus: "evaluation/",
    prompt: "broker/src/semantic/",
    sampling: "broker/src/semantic/",
  };
  const artifacts = [
    "broker",
    "adapter",
    "evaluator",
    "plugin",
    "policy",
    "corpus",
    "prompt",
    "sampling",
  ].map((role) => {
    const repositoryPath =
      role === "policy"
        ? "capabilities/v3/policy.mjs"
        : `${roots[role]}artifact.fixture`;
    return {
      id: role,
      role,
      locator: `repo:${repositoryPath}`,
      repository_path: repositoryPath,
      bytes: 1,
      sha256: digest,
    };
  });
  artifacts.push(
    {
      id: "model",
      role: "model",
      locator: "external:model",
      source_repository: "owner/model",
      source_artifact: "model.gguf",
      revision: "model-revision",
      license: "Apache-2.0",
      bytes: 1,
      sha256: digest,
    },
    {
      id: "backend",
      role: "backend",
      locator: "external:backend",
      source_repository: "owner/backend",
      source_artifact: "backend.tar.gz",
      revision: "backend-revision",
      license: "MIT",
      bytes: 1,
      sha256: digest,
    },
  );
  const gates = Object.entries(V3_GATE_CONTRACTS).map(
    ([id, contract]) => ({
      id,
      status: "pass",
      evidence_kind: contract.evidenceKind,
      evidence_refs: contract.refs.map(({ kind, id: evidenceId }) => ({
        kind,
        id: evidenceId,
      })),
    }),
  );
  const qualification = {
    policy: {
      identity: "badi.product-proof-policy.v3",
      artifact_id: "policy",
    },
    corpus: { identity: "badi.writing.en-v1", artifact_id: "corpus" },
    prompt: {
      identity: "badi.semantic.inline-en.native-prefix.dev1",
      artifact_id: "prompt",
    },
    evaluator: {
      identity: "badi.semantic-evaluator.v1",
      artifact_id: "evaluator",
    },
    sampling: {
      identity: "badi.semantic-sampling.dev1",
      artifact_id: "sampling",
    },
  };
  qualification.sha256 = computeV3QualificationSha256(
    qualification,
    artifacts,
  );
  return {
    status: "candidate",
    recorded_at: timestamp,
    repository,
    hardware,
    qualification,
    linked_evidence: evidence,
    artifacts,
    gates,
    approvals: [
      {
        phase: "pre-run-policy",
        role: "owner",
        status: "approved",
        recorded_at: timestamp,
        qualification_sha256: qualification.sha256,
        detail: "fixture policy approval",
      },
    ],
    rollback: { tested: false, processes_remaining: 0, config_remaining: 0 },
  };
}

test("V3 run semantics derive metric status", () => {
  const run = validRun();
  assert.doesNotThrow(() => assertV3RunSemantics("run.json", run));
  run.measurements[0].value = 251;
  assert.throws(
    () => assertV3RunSemantics("run.json", run),
    /measurement value is not derived/u,
  );
});

test("V3 measurements reject negative physical values but allow bounded signed deltas", () => {
  const run = validRun();
  run.measurements[0].observations = Array(1_000).fill(-1);
  run.measurements[0].value = -1;
  assert.throws(
    () => assertV3RunSemantics("run.json", run),
    /negative observation/u,
  );

  run.measurements = [
    {
      name: "semantic.phrase-v1-improvement",
      unit: "ratio",
      samples: 2,
      warmups: 0,
      statistic: "mean-difference",
      observations: [-0.2, 0.4],
      value: 0.1,
      threshold: 0.1,
      threshold_operator: "at-least",
      status: "pass",
    },
  ];
  assert.doesNotThrow(() => assertV3RunSemantics("run.json", run));
  run.measurements[0].observations[0] = -1.1;
  assert.throws(
    () => assertV3RunSemantics("run.json", run),
    /signed ratio/u,
  );
});

test("V3 run semantics reject a zero-observation passing check", () => {
  const run = validRun();
  run.checks[0].trials = 0;
  run.checks[0].passed = 0;
  assert.throws(
    () => assertV3RunSemantics("run.json", run),
    /has no observations/u,
  );
});

test("V3 receipt semantics require exact roles and gate mappings", () => {
  const receipt = validReceipt();
  assert.doesNotThrow(() => assertV3ReceiptSemantics("receipt.json", receipt));
  receipt.artifacts.push({
    id: "policy",
    role: "policy",
    locator: "repo:policy.fixture",
    repository_path: "policy.fixture",
    bytes: 1,
    sha256: digest,
  });
  assert.throws(
    () => assertV3ReceiptSemantics("receipt.json", receipt),
    /duplicate V3 artifact role/u,
  );
});

test("V3 receipt semantics reject a gate that borrows another reference", () => {
  const receipt = validReceipt();
  receipt.gates[0].evidence_refs[0].id = "semantic.cold-start-ms";
  assert.throws(
    () => assertV3ReceiptSemantics("receipt.json", receipt),
    /gate evidence mapping differs/u,
  );
});

test("V3 qualification binds approvals to exact policy and evaluation inputs", () => {
  const receipt = validReceipt();
  receipt.approvals[0].qualification_sha256 = "f".repeat(64);
  assert.throws(
    () => assertV3ReceiptSemantics("receipt.json", receipt),
    /approval qualification digest differs/u,
  );

  for (const name of ["corpus", "prompt", "evaluator", "sampling"]) {
    const invalid = validReceipt();
    invalid.qualification[name].artifact_id = "broker";
    assert.throws(
      () => assertV3ReceiptSemantics("receipt.json", invalid),
      new RegExp(`qualification ${name} artifact has role broker`, "u"),
    );
  }
});

test("V3 qualification rejects a coherent digest over a dummy policy artifact", () => {
  const receipt = validReceipt();
  const policy = receipt.artifacts.find(({ role }) => role === "policy");
  policy.repository_path = "capabilities/v3/policy-artifact.bin";
  policy.locator = `repo:${policy.repository_path}`;
  receipt.qualification.sha256 = computeV3QualificationSha256(
    receipt.qualification,
    receipt.artifacts,
  );
  receipt.approvals[0].qualification_sha256 = receipt.qualification.sha256;
  assert.throws(
    () => assertV3ReceiptSemantics("receipt.json", receipt),
    /policy artifact is not the V3 policy module/u,
  );
});

test("V3 linked runs cannot substitute another qualification digest", () => {
  const receipt = validReceipt();
  const link = receipt.linked_evidence.find(({ kind }) => kind === "semantic");
  const run = validRun();
  run.qualification_sha256 = receipt.qualification.sha256;
  assert.doesNotThrow(() =>
    assertV3LinkedRun("receipt.json", receipt, link, run),
  );
  run.qualification_sha256 = "f".repeat(64);
  assert.throws(
    () => assertV3LinkedRun("receipt.json", receipt, link, run),
    /evidence qualification digest differs/u,
  );
});

test("V3 timestamps reject normalized calendar dates and 24:00", () => {
  for (const recordedAt of [
    "2026-02-31T20:00:00Z",
    "2026-08-31T24:00:00Z",
  ]) {
    const receipt = validReceipt();
    receipt.recorded_at = recordedAt;
    assert.throws(
      () => assertV3ReceiptSemantics("receipt.json", receipt),
      /invalid V3 receipt timestamp/u,
    );

    const run = validRun();
    run.recorded_at = recordedAt;
    assert.throws(
      () => assertV3RunSemantics("run.json", run),
      /invalid V3 run timestamp/u,
    );
  }
});

test("V3 gate evidence rejects a weaker threshold or too few samples", () => {
  const gate = validReceipt().gates.find(
    ({ id }) => id === "semantic.ttft-p95",
  );
  const run = validRun();
  assert.doesNotThrow(() => assertV3GateEvidence("receipt.json", gate, run));
  run.measurements[0].threshold = 500;
  assert.throws(
    () => assertV3GateEvidence("receipt.json", gate, run),
    /weakens/u,
  );
  run.measurements[0].threshold = 250;
  run.measurements[0].samples = 999;
  assert.throws(
    () => assertV3GateEvidence("receipt.json", gate, run),
    /insufficient observations/u,
  );
});

test("V3 rates are derived from one binary numerator and denominator", () => {
  const run = validRun();
  run.measurements = [
    {
      name: "semantic.suggestion-rate",
      unit: "ratio",
      samples: 100,
      warmups: 0,
      statistic: "rate",
      observations: [...Array(5).fill(1), ...Array(95).fill(0)],
      numerator: 5,
      denominator: 100,
      value: 0.05,
      threshold_operator: "between",
      threshold: 0.05,
      threshold_max: 0.8,
      status: "pass",
    },
  ];
  assert.doesNotThrow(() => assertV3RunSemantics("run.json", run));
  run.measurements[0].numerator = 80;
  assert.throws(
    () => assertV3RunSemantics("run.json", run),
    /rate numerator\/denominator differs/u,
  );
});
