import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";

const EVIDENCE_KINDS = Object.freeze(["semantic", "chromium", "omarchy"]);
const ARTIFACT_ROLES = Object.freeze([
  "broker",
  "adapter",
  "evaluator",
  "plugin",
  "model",
  "backend",
  "policy",
  "corpus",
  "prompt",
  "sampling",
]);
const REPOSITORY_ARTIFACT_ROLES = Object.freeze([
  "broker",
  "adapter",
  "evaluator",
  "plugin",
  "policy",
  "corpus",
  "prompt",
  "sampling",
]);
const REPOSITORY_ROLE_ROOTS = Object.freeze({
  broker: "broker/",
  adapter: "adapters/chromium/",
  evaluator: "evaluation/",
  plugin: "ui/omarchy-plugin/",
  policy: "capabilities/v3/",
  corpus: "evaluation/",
  prompt: "broker/src/semantic/",
  sampling: "broker/src/semantic/",
});
const EXTERNAL_ARTIFACT_ROLES = Object.freeze(["model", "backend"]);
const QUALIFICATION_COMPONENTS = Object.freeze([
  Object.freeze({ name: "policy", role: "policy" }),
  Object.freeze({ name: "corpus", role: "corpus" }),
  Object.freeze({ name: "prompt", role: "prompt" }),
  Object.freeze({ name: "evaluator", role: "evaluator" }),
  Object.freeze({ name: "sampling", role: "sampling" }),
]);
const POLICY_IDENTITY = "badi.product-proof-policy.v3";
const POLICY_REPOSITORY_PATH = "capabilities/v3/policy.mjs";

const check = (id, minTrials, exactTrials = false) =>
  Object.freeze({ kind: "check", id, minTrials, exactTrials });
const attestation = (id) => Object.freeze({ kind: "manual_attestation", id });
const measurement = (
  id,
  unit,
  statistic,
  minSamples,
  minWarmups,
  thresholdOperator,
  threshold,
  thresholdMax,
  exactSamples = false,
) => Object.freeze({
  kind: "measurement",
  id,
  unit,
  statistic,
  minSamples,
  minWarmups,
  thresholdOperator,
  threshold,
  thresholdMax,
  exactSamples,
});

// This is the frozen product-proof policy, not merely a list of friendly
// labels. Every receipt gate resolves to observations with the exact units,
// statistics, sample floors, warmups, and non-weaker thresholds below.
export const V3_GATE_CONTRACTS = Object.freeze({
  "semantic.cases-100": {
    evidenceKind: "semantic",
    refs: [check("semantic.cases-100", 100)],
  },
  "semantic.cold-start": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.cold-start-ms", "ms", "maximum", 1, 0, "at-most", 10_000)],
  },
  "semantic.ttft-p95": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.ttft-p95-ms", "ms", "nearest-rank-p95", 1_000, 50, "at-most", 250)],
  },
  "semantic.visible-p50": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.schedule-to-visible-p50-ms", "ms", "nearest-rank-p50", 1_000, 50, "at-most", 250)],
  },
  "semantic.visible-p95": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.schedule-to-visible-p95-ms", "ms", "nearest-rank-p95", 1_000, 50, "at-most", 500)],
  },
  "semantic.stale-zero": {
    evidenceKind: "semantic",
    refs: [check("semantic.stale-zero", 100)],
  },
  "semantic.cancellation": {
    evidenceKind: "semantic",
    sharedObservations: true,
    refs: [
      measurement("semantic.cancellation-to-idle-p95-ms", "ms", "nearest-rank-p95", 100, 0, "at-most", 50),
      measurement("semantic.cancellation-to-idle-max-ms", "ms", "maximum", 100, 0, "at-most", 100),
    ],
  },
  "semantic.invalid-output": {
    evidenceKind: "semantic",
    refs: [
      measurement("semantic.invalid-output-rate", "ratio", "rate", 100, 0, "at-most", 0.01),
      measurement("semantic.truncated-output-rate", "ratio", "rate", 100, 0, "at-most", 0.01),
      measurement("semantic.late-output-rate", "ratio", "rate", 100, 0, "at-most", 0.01),
    ],
  },
  "semantic.visible-safety": {
    evidenceKind: "semantic",
    refs: [check("semantic.visible-safety-zero", 100)],
  },
  "semantic.quietness": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.quiet-false-shows", "count", "sum", 40, 0, "at-most", 2, undefined, true)],
  },
  "semantic.suggestion-rate": {
    evidenceKind: "semantic",
    refs: [
      measurement("semantic.suggestion-rate", "ratio", "rate", 100, 0, "between", 0.05, 0.8),
    ],
  },
  "semantic.usefulness": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.accepted-words-per-interruption", "words-per-interruption", "mean", 100, 0, "at-least", 1)],
  },
  "semantic.phrase-delta": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.phrase-v1-improvement", "ratio", "mean-difference", 100, 0, "at-least", 0.1)],
  },
  "semantic.blind-preference": {
    evidenceKind: "semantic",
    refs: [measurement("semantic.blind-preference-wins", "count", "sum", 40, 0, "at-least", 28, undefined, true)],
  },
  "semantic.memory": {
    evidenceKind: "semantic",
    refs: [
      measurement("semantic.process-tree-peak-rss-mib", "MiB", "maximum", 1, 0, "at-most", 4_096),
      measurement("semantic.swap-growth-mib", "MiB", "maximum", 1, 0, "equal", 0),
    ],
  },
  "chromium.insertion-1000": {
    evidenceKind: "chromium",
    refs: [check("chromium.insertion-1000", 1_000, true)],
  },
  "chromium.stale-100": {
    evidenceKind: "chromium",
    refs: [check("chromium.stale-100", 100, true)],
  },
  "chromium.accept-p95": {
    evidenceKind: "chromium",
    refs: [measurement("chromium.accept-to-verified-insert-p95-ms", "ms", "nearest-rank-p95", 1_000, 50, "at-most", 30)],
  },
  "chromium.hide-p95": {
    evidenceKind: "chromium",
    refs: [measurement("chromium.invalidation-to-hide-p95-ms", "ms", "nearest-rank-p95", 100, 0, "at-most", 32)],
  },
  "chromium.undo-100": {
    evidenceKind: "chromium",
    refs: [check("chromium.undo-100", 100, true)],
  },
  "chromium.privacy-zero": {
    evidenceKind: "chromium",
    refs: [
      check("chromium.scope-guard-zero-runtime-bytes", 4),
      check("chromium.adversarial-en-zero-display-insert", 4),
      check("chromium.corpus-canary-zero-leaks", 7),
      check("chromium.denied-sensitive-zero-bytes", 8),
    ],
  },
  "control.deny-durable": {
    evidenceKind: "omarchy",
    refs: [check("control.deny-durable", 1)],
  },
  "omarchy.validate": {
    evidenceKind: "omarchy",
    refs: [check("omarchy.validate", 2)],
  },
  "omarchy.lifecycle-100": {
    evidenceKind: "omarchy",
    refs: [check("omarchy.lifecycle-100", 100, true)],
  },
  "omarchy.themes": {
    evidenceKind: "omarchy",
    refs: [
      attestation("omarchy.theme.light"),
      attestation("omarchy.theme.dark-primary"),
      attestation("omarchy.theme.dark-secondary"),
      attestation("omarchy.theme.live-switch"),
    ],
  },
  "omarchy.accessibility": {
    evidenceKind: "omarchy",
    refs: [
      attestation("omarchy.accessibility.scale-2"),
      attestation("omarchy.accessibility.keyboard"),
      attestation("omarchy.accessibility.escape"),
      attestation("omarchy.accessibility.reduced-motion"),
      attestation("omarchy.accessibility.screen-reader"),
    ],
  },
});

function requireUnique(file, values, label) {
  if (new Set(values).size !== values.length) {
    throw new Error(`${file}: duplicate ${label}`);
  }
}

function requireExact(file, values, expected, label) {
  requireUnique(file, values, label);
  if (
    values.length !== expected.length ||
    expected.some((value) => !values.includes(value))
  ) {
    throw new Error(`${file}: ${label} set is not exact`);
  }
}

function requireTimestamp(file, value, label) {
  const match =
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?Z$/u.exec(
      value,
    );
  const parsed = new Date(value);
  if (
    match === null ||
    !Number.isFinite(parsed.getTime()) ||
    parsed.getUTCFullYear() !== Number(match[1]) ||
    parsed.getUTCMonth() + 1 !== Number(match[2]) ||
    parsed.getUTCDate() !== Number(match[3]) ||
    parsed.getUTCHours() !== Number(match[4]) ||
    parsed.getUTCMinutes() !== Number(match[5]) ||
    parsed.getUTCSeconds() !== Number(match[6])
  ) {
    throw new Error(`${file}: invalid ${label}`);
  }
}

function canonicalArtifact(artifact) {
  return {
    id: artifact.id,
    role: artifact.role,
    locator: artifact.locator,
    repository_path: artifact.repository_path ?? null,
    source_repository: artifact.source_repository ?? null,
    source_artifact: artifact.source_artifact ?? null,
    revision: artifact.revision ?? null,
    license: artifact.license ?? null,
    bytes: artifact.bytes,
    sha256: artifact.sha256,
  };
}

export function computeV3QualificationSha256(qualification, artifacts) {
  const artifactsById = new Map(
    artifacts.map((artifact) => [artifact.id, artifact]),
  );
  const components = QUALIFICATION_COMPONENTS.map(({ name, role }) => {
    const component = qualification[name];
    const artifact = artifactsById.get(component?.artifact_id);
    if (component === undefined || artifact === undefined) {
      throw new Error(`V3 qualification ${name} artifact is missing`);
    }
    if (artifact.role !== role) {
      throw new Error(`V3 qualification ${name} artifact has role ${artifact.role}`);
    }
    if (name === "policy" && component.identity !== POLICY_IDENTITY) {
      throw new Error("V3 qualification policy identity differs");
    }
    return {
      name,
      identity: component.identity,
      artifact: canonicalArtifact(artifact),
    };
  });
  return createHash("sha256")
    .update(
      JSON.stringify({
        contract: "badi.product-qualification.v3",
        components,
      }),
    )
    .digest("hex");
}

function numbersEqual(left, right) {
  const scale = Math.max(1, Math.abs(left), Math.abs(right));
  return Math.abs(left - right) <= Number.EPSILON * scale * 16;
}

function assertMeasurementDomain(file, measurement) {
  if (
    !Number.isFinite(measurement.value) ||
    !Number.isFinite(measurement.threshold) ||
    (measurement.threshold_max !== undefined &&
      !Number.isFinite(measurement.threshold_max)) ||
    measurement.observations.some((value) => !Number.isFinite(value))
  ) {
    throw new Error(`${file}: V3 measurement contains a non-finite number`);
  }
  if (measurement.statistic === "mean-difference") {
    if (
      measurement.unit !== "ratio" ||
      measurement.observations.some((value) => value < -1 || value > 1) ||
      measurement.value < -1 ||
      measurement.value > 1 ||
      measurement.threshold < -1 ||
      measurement.threshold > 1 ||
      (measurement.threshold_max !== undefined &&
        (measurement.threshold_max < -1 || measurement.threshold_max > 1))
    ) {
      throw new Error(
        `${file}: V3 mean-difference must be a signed ratio within [-1, 1]`,
      );
    }
    return;
  }
  if (
    measurement.observations.some((value) => value < 0) ||
    measurement.value < 0 ||
    measurement.threshold < 0 ||
    (measurement.threshold_max !== undefined && measurement.threshold_max < 0)
  ) {
    throw new Error(`${file}: V3 measurement contains a negative observation`);
  }
  if (
    measurement.unit === "ratio" &&
    (measurement.observations.some((value) => value > 1) ||
      measurement.value > 1 ||
      measurement.threshold > 1 ||
      (measurement.threshold_max !== undefined && measurement.threshold_max > 1))
  ) {
    throw new Error(`${file}: V3 ratio measurement is outside [0, 1]`);
  }
  if (
    (measurement.unit === "count" ||
      measurement.unit === "words-per-interruption") &&
    measurement.observations.some((value) => !Number.isSafeInteger(value))
  ) {
    throw new Error(`${file}: V3 count observation is not a safe integer`);
  }
}

function derivedMeasurementValue(file, measurement) {
  if (measurement.observations.length !== measurement.samples) {
    throw new Error(
      `${file}: V3 measurement sample count differs: ${measurement.name}`,
    );
  }
  assertMeasurementDomain(file, measurement);
  const observations = measurement.observations;
  const sum = observations.reduce((total, value) => total + value, 0);
  switch (measurement.statistic) {
    case "maximum":
      return observations.reduce((maximum, value) => Math.max(maximum, value));
    case "sum":
      return sum;
    case "mean":
    case "mean-difference":
      return sum / observations.length;
    case "rate": {
      if (observations.some((value) => value !== 0 && value !== 1)) {
        throw new Error(`${file}: V3 rate contains a non-binary observation`);
      }
      if (
        measurement.numerator === undefined ||
        measurement.denominator !== observations.length ||
        !numbersEqual(measurement.numerator, sum)
      ) {
        throw new Error(`${file}: V3 rate numerator/denominator differs`);
      }
      return sum / observations.length;
    }
    case "nearest-rank-p50":
    case "nearest-rank-p95": {
      const percentile = measurement.statistic.endsWith("p50") ? 0.5 : 0.95;
      const sorted = [...observations].sort((left, right) => left - right);
      return sorted[Math.ceil(percentile * sorted.length) - 1];
    }
    default:
      throw new Error(
        `${file}: unsupported V3 measurement statistic: ${measurement.statistic}`,
      );
  }
}

function measurementPassed(measurement) {
  if (measurement.threshold_operator === "at-most") {
    return measurement.value <= measurement.threshold;
  }
  if (measurement.threshold_operator === "at-least") {
    return measurement.value >= measurement.threshold;
  }
  if (measurement.threshold_operator === "between") {
    return (
      measurement.value >= measurement.threshold &&
      measurement.value <= measurement.threshold_max
    );
  }
  return measurement.value === measurement.threshold;
}

export function assertV3RunSemantics(file, run) {
  if (!run.id.startsWith(`badi-${run.kind}-run.`)) {
    throw new Error(`${file}: V3 run id does not match kind ${run.kind}`);
  }
  requireTimestamp(file, run.recorded_at, "V3 run timestamp");
  requireUnique(file, run.environment.components.map(({ name }) => name), "V3 component name");
  requireUnique(file, run.checks.map(({ id }) => id), "V3 check id");
  requireUnique(file, run.measurements.map(({ name }) => name), "V3 measurement name");
  requireUnique(file, run.artifacts.map(({ id }) => id), "V3 artifact id");
  requireUnique(
    file,
    run.manual_attestations.map(({ id }) => id),
    "V3 manual attestation id",
  );

  for (const check of run.checks) {
    if (check.passed > check.trials) {
      throw new Error(`${file}: V3 check passed count exceeds trials: ${check.id}`);
    }
    if (check.status === "pass" && check.passed !== check.trials) {
      throw new Error(`${file}: passing V3 check is not all-pass: ${check.id}`);
    }
    if (check.status === "pass" && check.trials === 0) {
      throw new Error(`${file}: passing V3 check has no observations: ${check.id}`);
    }
  }
  for (const measurement of run.measurements) {
    const derivedValue = derivedMeasurementValue(file, measurement);
    if (!numbersEqual(measurement.value, derivedValue)) {
      throw new Error(`${file}: V3 measurement value is not derived: ${measurement.name}`);
    }
    if ((measurement.status === "pass") !== measurementPassed(measurement)) {
      throw new Error(`${file}: V3 measurement status is not derived: ${measurement.name}`);
    }
  }
  for (const artifact of run.artifacts) {
    if (artifact.bytes === 0) {
      throw new Error(`${file}: V3 artifact is empty: ${artifact.id}`);
    }
    if (
      artifact.repository_path !== undefined &&
      artifact.locator !== `repo:${artifact.repository_path}`
    ) {
      throw new Error(`${file}: V3 repository artifact locator differs: ${artifact.id}`);
    }
  }
  for (const attestation of run.manual_attestations) {
    requireTimestamp(file, attestation.recorded_at, `V3 ${attestation.id} timestamp`);
    if (Date.parse(attestation.recorded_at) > Date.parse(run.recorded_at)) {
      throw new Error(`${file}: V3 run predates attestation: ${attestation.id}`);
    }
  }
  if (run.status === "pass") {
    if (run.checks.some(({ status }) => status !== "pass")) {
      throw new Error(`${file}: passing V3 run contains a non-passing check`);
    }
    if (run.measurements.some(({ status }) => status !== "pass")) {
      throw new Error(`${file}: passing V3 run contains a failing measurement`);
    }
    if (!run.isolation.temporary_tree_removed || run.isolation.processes_remaining !== 0) {
      throw new Error(`${file}: passing V3 run did not clean up its isolation`);
    }
  }
  if (/\/(?:home|Users)\//u.test(JSON.stringify(run))) {
    throw new Error(`${file}: V3 run contains an absolute personal path`);
  }
}

export function assertV3ReceiptSemantics(file, receipt) {
  requireTimestamp(file, receipt.recorded_at, "V3 receipt timestamp");
  requireExact(file, receipt.linked_evidence.map(({ kind }) => kind), EVIDENCE_KINDS, "V3 evidence kind");
  requireUnique(file, receipt.linked_evidence.map(({ id }) => id), "V3 evidence id");
  requireUnique(file, receipt.linked_evidence.map(({ path }) => path), "V3 evidence path");
  requireExact(file, receipt.artifacts.map(({ role }) => role), ARTIFACT_ROLES, "V3 artifact role");
  requireUnique(file, receipt.artifacts.map(({ id }) => id), "V3 artifact id");
  let qualificationSha256;
  try {
    qualificationSha256 = computeV3QualificationSha256(
      receipt.qualification,
      receipt.artifacts,
    );
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`${file}: ${detail}`);
  }
  if (receipt.qualification.sha256 !== qualificationSha256) {
    throw new Error(`${file}: V3 qualification digest differs`);
  }
  requireExact(
    file,
    receipt.gates.map(({ id }) => id),
    Object.keys(V3_GATE_CONTRACTS),
    "V3 gate",
  );
  requireUnique(
    file,
    receipt.approvals.map(({ phase, role }) => `${phase}:${role}`),
    "V3 approval phase/role",
  );

  for (const link of receipt.linked_evidence) {
    if (link.path !== `capabilities/evidence/${link.id}.json`) {
      throw new Error(`${file}: V3 evidence path does not match id: ${link.id}`);
    }
  }
  for (const gate of receipt.gates) {
    const contract = V3_GATE_CONTRACTS[gate.id];
    const expectedRefs = contract.refs.map(({ kind, id }) => ({ kind, id }));
    if (
      gate.evidence_kind !== contract.evidenceKind ||
      !isExactReferenceList(gate.evidence_refs, expectedRefs)
    ) {
      throw new Error(`${file}: V3 gate evidence mapping differs: ${gate.id}`);
    }
    if (receipt.status !== "unsupported" && gate.status !== "pass") {
      throw new Error(`${file}: V3 candidate/live receipt contains a non-passing gate`);
    }
  }
  for (const artifact of receipt.artifacts) {
    if (artifact.bytes === 0) {
      throw new Error(`${file}: V3 artifact is empty: ${artifact.id}`);
    }
    if (
      artifact.repository_path !== undefined &&
      artifact.locator !== `repo:${artifact.repository_path}`
    ) {
      throw new Error(`${file}: V3 repository artifact locator differs: ${artifact.id}`);
    }
    if (
      REPOSITORY_ARTIFACT_ROLES.includes(artifact.role) &&
      artifact.repository_path === undefined
    ) {
      throw new Error(`${file}: V3 repository artifact has no path: ${artifact.id}`);
    }
    if (
      REPOSITORY_ARTIFACT_ROLES.includes(artifact.role) &&
      !artifact.repository_path.startsWith(REPOSITORY_ROLE_ROOTS[artifact.role])
    ) {
      throw new Error(`${file}: V3 repository artifact has the wrong role root: ${artifact.id}`);
    }
    if (
      artifact.role === "policy" &&
      artifact.repository_path !== POLICY_REPOSITORY_PATH
    ) {
      throw new Error(`${file}: V3 policy artifact is not the V3 policy module`);
    }
    if (
      EXTERNAL_ARTIFACT_ROLES.includes(artifact.role) &&
      (artifact.repository_path !== undefined ||
        artifact.source_repository === undefined ||
        artifact.source_artifact === undefined ||
        artifact.revision === undefined ||
        artifact.license === undefined)
    ) {
      throw new Error(`${file}: V3 external artifact provenance is incomplete: ${artifact.id}`);
    }
  }
  for (const approval of receipt.approvals) {
    requireTimestamp(file, approval.recorded_at, `V3 ${approval.role} approval timestamp`);
    if (approval.qualification_sha256 !== qualificationSha256) {
      throw new Error(
        `${file}: V3 ${approval.role} approval qualification digest differs`,
      );
    }
    if (Date.parse(approval.recorded_at) > Date.parse(receipt.recorded_at)) {
      throw new Error(`${file}: V3 receipt predates ${approval.role} approval`);
    }
  }
  if (receipt.status !== "unsupported") {
    requireExact(
      file,
      receipt.approvals
        .filter(({ phase }) => phase === "pre-run-policy")
        .map(({ role }) => role),
      ["owner"],
      "V3 pre-run policy approval",
    );
  }
  if (receipt.status === "live") {
    requireExact(
      file,
      receipt.approvals
        .filter(({ phase }) => phase === "post-run-release")
        .map(({ role }) => role),
      ["owner", "omarchy-reviewer", "grillme-reviewer"],
      "V3 post-run release approval",
    );
    if (
      !receipt.rollback.tested ||
      receipt.rollback.processes_remaining !== 0 ||
      receipt.rollback.config_remaining !== 0
    ) {
      throw new Error(`${file}: V3 live rollback is incomplete`);
    }
  }
  if (/\/(?:home|Users)\//u.test(JSON.stringify(receipt))) {
    throw new Error(`${file}: V3 receipt contains an absolute personal path`);
  }
}

function isExactReferenceList(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected);
}

function assertMeasurementContract(file, gateId, observed, expected) {
  for (const [key, value] of [
    ["unit", expected.unit],
    ["statistic", expected.statistic],
    ["threshold_operator", expected.thresholdOperator],
    ["threshold", expected.threshold],
    ["threshold_max", expected.thresholdMax],
  ]) {
    if (observed[key] !== value) {
      throw new Error(`${file}: V3 gate ${gateId} weakens ${observed.name}.${key}`);
    }
  }
  if (
    (expected.exactSamples
      ? observed.samples !== expected.minSamples
      : observed.samples < expected.minSamples) ||
    observed.warmups < expected.minWarmups
  ) {
    throw new Error(`${file}: V3 gate ${gateId} has insufficient observations`);
  }
}

export function assertV3GateEvidence(file, gate, run) {
  const contract = V3_GATE_CONTRACTS[gate.id];
  if (contract === undefined || run.kind !== contract.evidenceKind) {
    throw new Error(`${file}: V3 gate kind differs: ${gate.id}`);
  }
  const observedMeasurements = [];
  for (const expected of contract.refs) {
    const observed = (expected.kind === "check"
      ? run.checks
      : expected.kind === "measurement"
        ? run.measurements
        : run.manual_attestations)
      .find(({ id, name }) => (id ?? name) === expected.id);
    if (observed === undefined || observed.status !== gate.status) {
      throw new Error(`${file}: V3 gate evidence missing or differs: ${gate.id}`);
    }
    if (expected.kind === "check") {
      if (
        (expected.exactTrials
          ? observed.trials !== expected.minTrials
          : observed.trials < expected.minTrials) ||
        observed.passed !== observed.trials
      ) {
        throw new Error(`${file}: V3 gate ${gate.id} has insufficient passing trials`);
      }
    } else if (expected.kind === "measurement") {
      assertMeasurementContract(file, gate.id, observed, expected);
      observedMeasurements.push(observed);
    }
  }
  if (
    contract.sharedObservations === true &&
    observedMeasurements.some(
      (measurement) =>
        !isExactReferenceList(
          measurement.observations,
          observedMeasurements[0].observations,
        ),
    )
  ) {
    throw new Error(`${file}: V3 gate ${gate.id} does not share one sample set`);
  }
}

function requireV3Component(
  file,
  run,
  name,
  version,
  sourceIdentity,
  declaredMinimum,
) {
  const components = run.environment.components.filter(
    (component) => component.name === name,
  );
  if (
    components.length !== 1 ||
    components[0].version !== version ||
    (sourceIdentity !== undefined &&
      components[0].source_identity !== sourceIdentity) ||
    (declaredMinimum !== undefined &&
      components[0].declared_minimum !== declaredMinimum)
  ) {
    throw new Error(`${file}: V3 ${run.kind} component differs: ${name}`);
  }
}

function requireV3ArtifactRoles(file, run, receiptArtifacts, roles) {
  for (const role of roles) {
    const expected = receiptArtifacts.get(role);
    if (
      expected === undefined ||
      !run.artifacts.some((artifact) => isDeepStrictEqual(artifact, expected))
    ) {
      throw new Error(`${file}: V3 ${run.kind} run does not bind ${role}`);
    }
  }
}

function assertV3Compatibility(file, receipt, linkedRuns) {
  const { target, browser, desktop, model } = receipt.compatibility;
  const expectedTargetScope = {
    claim: receipt.claim.interaction,
    target_app: target.app,
    target: target.origin,
    target_version: target.version,
    surface: target.surface,
    editor_api: target.editor_api,
    language: receipt.claim.language,
    display_route: target.display_route,
    insertion_route: target.insertion_route,
  };
  for (const kind of ["semantic", "chromium"]) {
    const run = linkedRuns.get(kind);
    for (const [key, value] of Object.entries(expectedTargetScope)) {
      if (run.scope[key] !== value) {
        throw new Error(`${file}: V3 ${kind} scope differs: ${key}`);
      }
    }
  }
  if (target.commit_status !== "applied") {
    throw new Error(`${file}: V3 product cell lacks verified target insertion`);
  }

  const semanticRun = linkedRuns.get("semantic");
  const chromiumRun = linkedRuns.get("chromium");
  const omarchyRun = linkedRuns.get("omarchy");
  requireV3Component(file, semanticRun, "llama.cpp", model.backend_revision);
  requireV3Component(
    file,
    chromiumRun,
    "chromium",
    browser.tested_version,
    undefined,
    browser.declared_minimum,
  );
  requireV3Component(
    file,
    omarchyRun,
    "omarchy",
    desktop.omarchy,
    desktop.omarchy_source,
  );
  for (const [name, version] of [
    ["quickshell", desktop.quickshell],
    ["hyprland", desktop.hyprland],
    ["qt", desktop.qt],
  ]) {
    requireV3Component(file, omarchyRun, name, version);
  }
  if (
    omarchyRun.scope.claim !== "Badi control panel" ||
    omarchyRun.scope.target_app !== "Omarchy" ||
    omarchyRun.scope.target !== "omarchy-shell" ||
    omarchyRun.scope.target_version !== desktop.omarchy ||
    omarchyRun.scope.surface !== "panel" ||
    omarchyRun.scope.editor_api !== "not-applicable" ||
    omarchyRun.scope.display_route !== "omarchy-shell-plugin" ||
    omarchyRun.scope.insertion_route !== "not-applicable"
  ) {
    throw new Error(`${file}: V3 Omarchy scope is not the exact panel contract`);
  }
  if (
    !isDeepStrictEqual(omarchyRun.environment.themes, desktop.themes) ||
    desktop.monitor_cell !== receipt.hardware.cell_id
  ) {
    throw new Error(`${file}: V3 Omarchy theme/monitor cell differs`);
  }

  const artifacts = new Map(
    receipt.artifacts.map((artifact) => [artifact.role, artifact]),
  );
  requireV3ArtifactRoles(file, semanticRun, artifacts, [
    "broker",
    "evaluator",
    "model",
    "backend",
    "policy",
    "corpus",
    "prompt",
    "sampling",
  ]);
  requireV3ArtifactRoles(file, chromiumRun, artifacts, [
    "broker",
    "adapter",
    "model",
    "backend",
  ]);
  requireV3ArtifactRoles(file, omarchyRun, artifacts, ["broker", "plugin"]);

  const modelArtifact = artifacts.get("model");
  const backendArtifact = artifacts.get("backend");
  if (
    modelArtifact.source_repository !== model.repository ||
    modelArtifact.source_artifact !== model.artifact ||
    modelArtifact.revision !== model.revision ||
    modelArtifact.license !== model.license ||
    modelArtifact.sha256 !== model.sha256 ||
    backendArtifact.source_repository !== model.backend ||
    backendArtifact.source_artifact !== model.backend_artifact ||
    backendArtifact.revision !== model.backend_revision ||
    backendArtifact.sha256 !== model.backend_sha256
  ) {
    throw new Error(`${file}: V3 model/backend provenance differs`);
  }
}

export function assertV3LinkedRun(file, receipt, link, run) {
  assertV3RunSemantics(link.path, run);
  if (run.id !== link.id || run.kind !== link.kind || run.status !== "pass") {
    throw new Error(`${file}: V3 evidence identity/status differs: ${link.path}`);
  }
  if (!isDeepStrictEqual(run.repository, receipt.repository)) {
    throw new Error(`${file}: V3 evidence repository differs: ${link.path}`);
  }
  if (!isDeepStrictEqual(run.hardware, receipt.hardware)) {
    throw new Error(`${file}: V3 evidence hardware differs: ${link.path}`);
  }
  if (run.qualification_sha256 !== receipt.qualification.sha256) {
    throw new Error(
      `${file}: V3 evidence qualification digest differs: ${link.path}`,
    );
  }
  if (Date.parse(run.recorded_at) > Date.parse(receipt.recorded_at)) {
    throw new Error(`${file}: V3 receipt predates evidence: ${link.path}`);
  }
}

export function assertV3ProductCell(file, receipt, linkedRuns) {
  const runTimes = [...linkedRuns.values()].map(({ recorded_at }) =>
    Date.parse(recorded_at),
  );
  const earliestRun = Math.min(...runTimes);
  const latestRun = Math.max(...runTimes);
  for (const approval of receipt.approvals) {
    const approvalTime = Date.parse(approval.recorded_at);
    if (
      (approval.phase === "pre-run-policy" && approvalTime > earliestRun) ||
      (approval.phase === "post-run-release" && approvalTime < latestRun)
    ) {
      throw new Error(
        `${file}: V3 ${approval.phase} approval is on the wrong side of evidence`,
      );
    }
  }

  for (const gate of receipt.gates) {
    assertV3GateEvidence(file, gate, linkedRuns.get(gate.evidence_kind));
  }
  assertV3Compatibility(file, receipt, linkedRuns);

  const modelArtifact = receipt.artifacts.find(
    (artifact) => artifact.role === "model",
  );
  const backendArtifact = receipt.artifacts.find(
    (artifact) => artifact.role === "backend",
  );
  if (
    modelArtifact?.sha256 !== receipt.compatibility.model.sha256 ||
    backendArtifact?.sha256 !== receipt.compatibility.model.backend_sha256
  ) {
    throw new Error(`${file}: V3 model/backend identity differs from artifacts`);
  }
  const runArtifacts = [...linkedRuns.values()].flatMap((run) => run.artifacts);
  for (const artifact of receipt.artifacts) {
    if (!runArtifacts.some((candidate) => isDeepStrictEqual(candidate, artifact))) {
      throw new Error(
        `${file}: V3 artifact is not linked by a raw run: ${artifact.id}`,
      );
    }
  }
}
