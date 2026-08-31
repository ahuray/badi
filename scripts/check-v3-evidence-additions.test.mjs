import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import {
  appendFile,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import test from "node:test";
import { promisify } from "node:util";
import {
  V3_GATE_CONTRACTS,
  computeV3QualificationSha256,
} from "../capabilities/v3/policy.mjs";

const execFileAsync = promisify(execFile);
const sourceRepository = path.resolve(import.meta.dirname, "..");
const timestamp = Object.freeze({
  approval: "2026-08-31T19:00:00Z",
  run: "2026-08-31T20:00:00Z",
  release: "2026-08-31T21:00:00Z",
  receipt: "2026-08-31T22:00:00Z",
});
const hardware = Object.freeze({
  cell_id: "fixture-cell",
  cpu: "fixture-cpu",
  logical_cpus: 4,
  ram_mib: 8192,
  gpu: "fixture-gpu",
  stable_identity_sha256: "c".repeat(64),
  displays: [{ name: "fixture-display", width: 1920, height: 1080, scale: 1 }],
});
const compatibility = Object.freeze({
  target: {
    app: "Fixture Editor",
    version: "1.0",
    origin: "https://fixture.invalid",
    surface: "textarea",
    editor_api: "HTMLTextAreaElement",
    display_route: "overlay",
    insertion_route: "native-input",
    commit_status: "applied",
  },
  browser: {
    name: "Chromium",
    tested_version: "140.0",
    declared_minimum: "132",
    headed: true,
  },
  desktop: {
    omarchy: "3.2.1",
    omarchy_source: "omarchy@fixture",
    quickshell: "0.2.0",
    hyprland: "0.50.1",
    qt: "6.9.1",
    themes: ["light", "dark-primary", "dark-secondary"],
    monitor_cell: hardware.cell_id,
  },
  model: {
    repository: "https://models.invalid/fixture",
    revision: "model-revision",
    artifact: "fixture-model.gguf",
    license: "MIT",
    sha256: "a".repeat(64),
    backend: "https://backends.invalid/llama.cpp",
    backend_revision: "backend-revision",
    backend_artifact: "llama-server",
    backend_sha256: "b".repeat(64),
  },
});

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function serialize(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

async function run(file, arguments_, options = {}) {
  return execFileAsync(file, arguments_, {
    cwd: options.cwd,
    encoding: "utf8",
    env: options.env ?? process.env,
    maxBuffer: 32 * 1024 * 1024,
    timeout: 60_000,
  });
}

async function git(repository, ...arguments_) {
  return run("git", arguments_, { cwd: repository });
}

async function commit(repository, message) {
  await git(repository, "add", "--all");
  await git(repository, "commit", "--quiet", "-m", message);
  const { stdout } = await git(repository, "rev-parse", "HEAD");
  return stdout.trim();
}

async function unrelatedCommit(repository) {
  const { stdout: tree } = await git(repository, "rev-parse", "HEAD^{tree}");
  const { stdout } = await git(
    repository,
    "commit-tree",
    tree.trim(),
    "-m",
    "unrelated comparison base",
  );
  return stdout.trim();
}

async function copySource(repository, relativePath) {
  const destination = path.join(repository, relativePath);
  await mkdir(path.dirname(destination), { recursive: true });
  await copyFile(path.join(sourceRepository, relativePath), destination);
}

function desiredObservation(reference) {
  if (reference.thresholdOperator === "at-most") return 0;
  if (reference.thresholdOperator === "between") {
    return (reference.threshold + reference.thresholdMax) / 2;
  }
  if (reference.thresholdOperator === "at-least") {
    return reference.threshold + Math.max(1, Math.abs(reference.threshold)) * 0.1;
  }
  return reference.threshold;
}

function measurementFor(reference) {
  const samples = reference.minSamples;
  const desired =
    reference.unit === "count" || reference.unit === "words-per-interruption"
      ? Math.ceil(desiredObservation(reference))
      : desiredObservation(reference);
  let observations;
  if (reference.statistic === "rate") {
    const target = desired;
    const ones = Math.ceil(target * samples);
    observations = Array.from({ length: samples }, (_, index) =>
      index < ones ? 1 : 0,
    );
  } else if (reference.statistic === "sum") {
    observations = [desired, ...Array(samples - 1).fill(0)];
  } else {
    observations = Array(samples).fill(desired);
  }
  const sum = observations.reduce((total, value) => total + value, 0);
  let value;
  if (reference.statistic === "maximum") {
    value = Math.max(...observations);
  } else if (reference.statistic === "sum") {
    value = sum;
  } else if (
    reference.statistic === "mean" ||
    reference.statistic === "mean-difference" ||
    reference.statistic === "rate"
  ) {
    value = sum / observations.length;
  } else {
    const percentile = reference.statistic.endsWith("p50") ? 0.5 : 0.95;
    value = [...observations].sort((left, right) => left - right)[
      Math.ceil(percentile * observations.length) - 1
    ];
  }
  return {
    name: reference.id,
    unit: reference.unit,
    samples,
    warmups: reference.minWarmups,
    statistic: reference.statistic,
    observations,
    ...(reference.statistic === "rate"
      ? { numerator: sum, denominator: samples }
      : {}),
    value,
    threshold_operator: reference.thresholdOperator,
    threshold: reference.threshold,
    ...(reference.thresholdMax === undefined
      ? {}
      : { threshold_max: reference.thresholdMax }),
    status: "pass",
  };
}

function evidenceFor(kind) {
  const checks = new Map();
  const measurements = new Map();
  const attestations = new Map();
  for (const contract of Object.values(V3_GATE_CONTRACTS)) {
    if (contract.evidenceKind !== kind) continue;
    for (const reference of contract.refs) {
      if (reference.kind === "check" && !checks.has(reference.id)) {
        checks.set(reference.id, {
          id: reference.id,
          status: "pass",
          trials: reference.minTrials,
          passed: reference.minTrials,
          detail: "fixture check passed",
        });
      } else if (
        reference.kind === "measurement" &&
        !measurements.has(reference.id)
      ) {
        measurements.set(reference.id, measurementFor(reference));
      } else if (
        reference.kind === "manual_attestation" &&
        !attestations.has(reference.id)
      ) {
        attestations.set(reference.id, {
          id: reference.id,
          status: "pass",
          recorded_at: timestamp.run,
          detail: "fixture attestation passed",
        });
      }
    }
  }
  return {
    checks: [...checks.values()],
    measurements: [...measurements.values()],
    manual_attestations: [...attestations.values()],
  };
}

function scopeFor(kind) {
  if (kind === "omarchy") {
    return {
      claim: "Badi control panel",
      target_app: "Omarchy",
      target: "omarchy-shell",
      target_version: compatibility.desktop.omarchy,
      surface: "panel",
      editor_api: "not-applicable",
      language: "en",
      display_route: "omarchy-shell-plugin",
      insertion_route: "not-applicable",
    };
  }
  return {
    claim: "bounded plain-text suffix at a collapsed caret",
    target_app: compatibility.target.app,
    target: compatibility.target.origin,
    target_version: compatibility.target.version,
    surface: compatibility.target.surface,
    editor_api: compatibility.target.editor_api,
    language: "en",
    display_route: compatibility.target.display_route,
    insertion_route: compatibility.target.insertion_route,
  };
}

function environmentFor(kind) {
  if (kind === "semantic") {
    return {
      os: "Fixture Linux",
      kernel: "fixture-kernel",
      components: [
        { name: "llama.cpp", version: compatibility.model.backend_revision },
      ],
    };
  }
  if (kind === "chromium") {
    return {
      os: "Fixture Linux",
      kernel: "fixture-kernel",
      components: [
        {
          name: "chromium",
          version: compatibility.browser.tested_version,
          declared_minimum: compatibility.browser.declared_minimum,
        },
      ],
    };
  }
  return {
    os: "Fixture Linux",
    kernel: "fixture-kernel",
    components: [
      {
        name: "omarchy",
        version: compatibility.desktop.omarchy,
        source_identity: compatibility.desktop.omarchy_source,
      },
      { name: "quickshell", version: compatibility.desktop.quickshell },
      { name: "hyprland", version: compatibility.desktop.hyprland },
      { name: "qt", version: compatibility.desktop.qt },
    ],
    themes: compatibility.desktop.themes,
  };
}

function qualificationFor(artifacts) {
  const artifactsByRole = new Map(
    artifacts.map((artifact) => [artifact.role, artifact]),
  );
  const qualification = {
    policy: {
      identity: "badi.product-proof-policy.v3",
      artifact_id: artifactsByRole.get("policy").id,
    },
    corpus: {
      identity: "badi.writing.en-v1",
      artifact_id: artifactsByRole.get("corpus").id,
    },
    prompt: {
      identity: "badi.semantic.inline-en.native-prefix.dev1",
      artifact_id: artifactsByRole.get("prompt").id,
    },
    evaluator: {
      identity: "badi.semantic-evaluator.v1",
      artifact_id: artifactsByRole.get("evaluator").id,
    },
    sampling: {
      identity: "badi.semantic-sampling.dev1",
      artifact_id: artifactsByRole.get("sampling").id,
    },
  };
  qualification.sha256 = computeV3QualificationSha256(
    qualification,
    artifacts,
  );
  return qualification;
}

function buildRun(
  kind,
  suffix,
  repositoryCommit,
  artifacts,
  qualificationSha256,
) {
  const roles = {
    semantic: [
      "broker",
      "evaluator",
      "model",
      "backend",
      "policy",
      "corpus",
      "prompt",
      "sampling",
    ],
    chromium: ["broker", "adapter", "model", "backend"],
    omarchy: ["broker", "plugin"],
  }[kind];
  return {
    $schema: "../v3/run.schema.json",
    record_version: 3,
    id: `badi-${kind}-run.${suffix}.v3`,
    kind,
    status: "pass",
    recorded_at: timestamp.run,
    repository: { commit: repositoryCommit, working_tree_dirty: false },
    qualification_sha256: qualificationSha256,
    environment: environmentFor(kind),
    hardware,
    scope: scopeFor(kind),
    isolation: {
      temporary_home: true,
      temporary_xdg: true,
      temporary_browser_profile: true,
      real_browser_profile_touched: false,
      user_config_touched: false,
      system_files_touched: false,
      temporary_tree_removed: true,
      processes_remaining: 0,
    },
    privacy: {
      context_text_recorded: false,
      suggestion_text_recorded: false,
      prompt_or_token_text_logged: false,
      context_fingerprints_recorded: false,
      absolute_personal_paths_recorded: false,
      secrets_recorded: false,
    },
    ...evidenceFor(kind),
    artifacts: artifacts.filter((artifact) => roles.includes(artifact.role)),
    exclusions: [],
    notes: [],
  };
}

function approvalsFor(status, qualificationSha256) {
  const approvals = [
    {
      phase: "pre-run-policy",
      role: "owner",
      status: "approved",
      recorded_at: timestamp.approval,
      qualification_sha256: qualificationSha256,
      detail: "fixture policy approved",
    },
  ];
  if (status === "live") {
    for (const role of ["owner", "omarchy-reviewer", "grillme-reviewer"]) {
      approvals.push({
        phase: "post-run-release",
        role,
        status: "approved",
        recorded_at: timestamp.release,
        qualification_sha256: qualificationSha256,
        detail: "fixture release approved",
      });
    }
  }
  return approvals;
}

async function addReceipt(
  repository,
  suffix,
  status,
  repositoryCommit,
  artifacts,
) {
  const qualification = qualificationFor(artifacts);
  const linkedEvidence = [];
  for (const kind of ["semantic", "chromium", "omarchy"]) {
    const runValue = buildRun(
      kind,
      suffix,
      repositoryCommit,
      artifacts,
      qualification.sha256,
    );
    const bytes = Buffer.from(serialize(runValue));
    const relativePath = `capabilities/evidence/${runValue.id}.json`;
    await writeFile(path.join(repository, relativePath), bytes);
    linkedEvidence.push({
      kind,
      id: runValue.id,
      path: relativePath,
      bytes: bytes.byteLength,
      sha256: digest(bytes),
    });
  }
  const id = `badi-product-cell.${suffix}.v3`;
  const receipt = {
    $schema: "./v3/product-cell.schema.json",
    record_version: 3,
    id,
    status,
    recorded_at: timestamp.receipt,
    repository: { commit: repositoryCommit, working_tree_dirty: false },
    qualification,
    claim: {
      product: "Badi",
      language: "en",
      interaction: "bounded plain-text suffix at a collapsed caret",
      locality: "local-only",
    },
    compatibility,
    hardware,
    linked_evidence: linkedEvidence,
    artifacts,
    gates: Object.entries(V3_GATE_CONTRACTS).map(([gateId, contract]) => ({
      id: gateId,
      status: "pass",
      evidence_kind: contract.evidenceKind,
      evidence_refs: contract.refs.map(({ kind, id: referenceId }) => ({
        kind,
        id: referenceId,
      })),
    })),
    approvals: approvalsFor(status, qualification.sha256),
    exclusions: [],
    rollback: {
      tested: status === "live",
      detail: "fixture rollback state",
      processes_remaining: 0,
      config_remaining: 0,
    },
    notes: [],
  };
  await writeFile(
    path.join(repository, `capabilities/${id}.json`),
    serialize(receipt),
  );
  return { id, linkedEvidence };
}

async function createArtifacts(repository, brokerKind) {
  const repositoryArtifacts = [
    ["broker", "broker/broker.bin", "fixture broker\n"],
    ["adapter", "adapters/chromium/adapter.bin", "fixture adapter\n"],
    ["evaluator", "evaluation/src/runner.rs", "fixture evaluator\n"],
    ["plugin", "ui/omarchy-plugin/plugin.bin", "fixture plugin\n"],
    ["policy", "capabilities/v3/policy.mjs", null],
    ["corpus", "evaluation/writing/en-v1/corpus.json", "fixture corpus\n"],
    [
      "prompt",
      "broker/src/semantic/client.rs",
      "fixture prompt and sampling contract\n",
    ],
    [
      "sampling",
      "broker/src/semantic/client.rs",
      "fixture prompt and sampling contract\n",
    ],
  ];
  const artifacts = [];
  for (const [role, relativePath, content] of repositoryArtifacts) {
    const absolutePath = path.join(repository, relativePath);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    let bytes = content === null ? await readFile(absolutePath) : Buffer.from(content);
    if (role === "broker" && brokerKind === "symlink") {
      const target = "broker-target.bin";
      await writeFile(path.join(path.dirname(absolutePath), target), content);
      await symlink(target, absolutePath);
      bytes = Buffer.from(target);
    } else if (role === "broker" && brokerKind === "directory") {
      await mkdir(absolutePath, { recursive: true });
      await writeFile(path.join(absolutePath, "nested.bin"), content);
      bytes = Buffer.from("directory fixture");
    } else if (content !== null) {
      await writeFile(absolutePath, bytes);
    }
    artifacts.push({
      id: `${role}-artifact`,
      role,
      locator: `repo:${relativePath}`,
      repository_path: relativePath,
      bytes: bytes.byteLength,
      sha256: digest(bytes),
    });
  }
  artifacts.push(
    {
      id: "model-artifact",
      role: "model",
      locator: "model:fixture-model.gguf",
      source_repository: compatibility.model.repository,
      source_artifact: compatibility.model.artifact,
      revision: compatibility.model.revision,
      license: compatibility.model.license,
      bytes: 1024,
      sha256: compatibility.model.sha256,
    },
    {
      id: "backend-artifact",
      role: "backend",
      locator: "backend:llama-server",
      source_repository: compatibility.model.backend,
      source_artifact: compatibility.model.backend_artifact,
      revision: compatibility.model.backend_revision,
      license: "MIT",
      bytes: 2048,
      sha256: compatibility.model.backend_sha256,
    },
  );
  return artifacts;
}

async function createFixture({
  brokerKind = "regular",
  receipts = [{ suffix: "fixture", status: "candidate" }],
} = {}) {
  const repository = await mkdtemp(
    path.join(sourceRepository, "node_modules/.badi-v3-evidence-"),
  );
  try {
    for (const relativePath of [
      "scripts/check-capabilities.mjs",
      "scripts/check-capability-immutability.mjs",
      "scripts/check-v3-evidence-additions.mjs",
      "capabilities/v2/manifest-policy.mjs",
      "capabilities/v3/policy.mjs",
      "capabilities/v3/validator.mjs",
      "capabilities/v3/product-cell.schema.json",
      "capabilities/v3/run.schema.json",
    ]) {
      await copySource(repository, relativePath);
    }
    await mkdir(path.join(repository, "capabilities/evidence"), {
      recursive: true,
    });
    await mkdir(path.join(repository, "src"), { recursive: true });
    await writeFile(
      path.join(repository, "src/implementation.txt"),
      "clean implementation\n",
    );
    const artifacts = await createArtifacts(repository, brokerKind);
    await git(repository, "init", "--quiet");
    await git(repository, "config", "user.name", "Badi V3 Test");
    await git(repository, "config", "user.email", "badi-v3@example.invalid");
    const implementationCommit = await commit(repository, "implementation");
    const addedReceipts = [];
    for (const receipt of receipts) {
      addedReceipts.push(
        await addReceipt(
          repository,
          receipt.suffix,
          receipt.status,
          implementationCommit,
          artifacts,
        ),
      );
    }
    await commit(repository, "append-only V3 evidence");
    return {
      addedReceipts,
      implementationCommit,
      repository,
      async cleanup() {
        await rm(repository, { recursive: true, force: true });
      },
    };
  } catch (error) {
    await rm(repository, { recursive: true, force: true });
    throw error;
  }
}

async function runAdditionsCheck(fixture, base = fixture.implementationCommit) {
  return run(process.execPath, ["scripts/check-v3-evidence-additions.mjs"], {
    cwd: fixture.repository,
    env: { ...process.env, CAPABILITY_BASE_SHA: base },
  });
}

async function runAdditionsCheckWithoutBase(fixture) {
  const env = { ...process.env };
  delete env.CAPABILITY_BASE_SHA;
  return run(process.execPath, ["scripts/check-v3-evidence-additions.mjs"], {
    cwd: fixture.repository,
    env,
  });
}

async function runImmutabilityCheck(fixture, base) {
  return run(process.execPath, ["scripts/check-capability-immutability.mjs"], {
    cwd: fixture.repository,
    env: { ...process.env, CAPABILITY_BASE_SHA: base },
  });
}

async function expectFailure(promise) {
  try {
    await promise;
    assert.fail("command unexpectedly succeeded");
  } catch (error) {
    if (error instanceof assert.AssertionError) throw error;
    return `${error.stdout ?? ""}${error.stderr ?? ""}`;
  }
}

test("strict-current accepts append-only candidate and live V3 evidence", {
  timeout: 120_000,
}, async () => {
  const fixture = await createFixture({
    receipts: [
      { suffix: "fixture-candidate", status: "candidate" },
      { suffix: "fixture-live", status: "live" },
    ],
  });
  try {
    const { stdout } = await runAdditionsCheck(fixture);
    assert.match(stdout, /Validated 2 new V3 product-cell receipt\(s\)/u);
    assert.match(stdout, /selected=badi-product-cell\.fixture-candidate\.v3/u);
    assert.match(stdout, /selected=badi-product-cell\.fixture-live\.v3/u);

    const { stdout: immutableOutput } = await runImmutabilityCheck(
      fixture,
      fixture.implementationCommit,
    );
    assert.match(immutableOutput, /Capability evidence is immutable/u);

    const unsafeOutput = await expectFailure(runAdditionsCheck(fixture, "HEAD.."));
    assert.match(unsafeOutput, /must be set to a safe Git ref/u);

    const missingOutput = await expectFailure(runAdditionsCheckWithoutBase(fixture));
    assert.match(missingOutput, /must be set to a safe Git ref/u);
  } finally {
    await fixture.cleanup();
  }
});

test("capability comparison gates reject self and all-zero bases", async () => {
  const fixture = await createFixture();
  try {
    const unrelatedBase = await unrelatedCommit(fixture.repository);
    for (const runCheck of [runAdditionsCheck, runImmutabilityCheck]) {
      const selfOutput = await expectFailure(runCheck(fixture, "HEAD"));
      assert.match(selfOutput, /must identify a commit before HEAD/u);

      const zeroOutput = await expectFailure(runCheck(fixture, "0".repeat(40)));
      assert.match(zeroOutput, /all-zero push sentinel/u);

      const unrelatedOutput = await expectFailure(
        runCheck(fixture, unrelatedBase),
      );
      assert.match(unrelatedOutput, /not an ancestor of HEAD/u);
    }
  } finally {
    await fixture.cleanup();
  }
});

test("strict-current rejects source drift after the implementation commit", {
  timeout: 120_000,
}, async () => {
  const fixture = await createFixture();
  try {
    await writeFile(
      path.join(fixture.repository, "src/implementation.txt"),
      "drifted implementation\n",
    );
    await commit(fixture.repository, "source drift");
    const output = await expectFailure(runAdditionsCheck(fixture));
    assert.match(output, /non-evidence change follows the implementation commit/u);
  } finally {
    await fixture.cleanup();
  }
});

test("strict-current rejects modified evidence", { timeout: 120_000 }, async () => {
  const fixture = await createFixture();
  try {
    const evidencePath = fixture.addedReceipts[0].linkedEvidence[0].path;
    await appendFile(path.join(fixture.repository, evidencePath), "\n");
    await commit(fixture.repository, "tamper with evidence");
    const output = await expectFailure(runAdditionsCheck(fixture));
    assert.match(output, /V3 evidence artifact differs/u);
  } finally {
    await fixture.cleanup();
  }
});

test("historical validation rejects V3 schema substitution", {
  timeout: 120_000,
}, async () => {
  const fixture = await createFixture();
  try {
    const checkerPath = path.join(
      fixture.repository,
      "scripts/check-capabilities.mjs",
    );
    const checker = await readFile(checkerPath, "utf8");
    const hostileChecker = checker.replace(
      'const legacyEvidenceSchemaByVersion = new Map([\n  [1, "../v2/live-run.schema.json"],',
      'const legacyEvidenceSchemaByVersion = new Map([\n  [1, "../v2/live-run.schema.json"],\n  [3, "../v3/product-cell.schema.json"],',
    );
    assert.notEqual(hostileChecker, checker);
    await writeFile(checkerPath, hostileChecker);
    const evidencePath = fixture.addedReceipts[0].linkedEvidence[0].path;
    const evidence = JSON.parse(
      await readFile(path.join(fixture.repository, evidencePath), "utf8"),
    );
    evidence.$schema = "../v3/product-cell.schema.json";
    await writeFile(path.join(fixture.repository, evidencePath), serialize(evidence));
    await commit(fixture.repository, "substitute raw evidence schema");
    const output = await expectFailure(runAdditionsCheck(fixture));
    assert.match(output, /schema identity differs for record_version 3/u);
  } finally {
    await fixture.cleanup();
  }
});

test("protected V3 dispatch rejects semantics despite hostile legacy routing", {
  timeout: 120_000,
}, async () => {
  const fixture = await createFixture();
  try {
    const checkerPath = path.join(
      fixture.repository,
      "scripts/check-capabilities.mjs",
    );
    const checker = await readFile(checkerPath, "utf8");
    const hostileChecker = checker
      .replace(
        'const legacyEvidenceSchemaByVersion = new Map([\n  [1, "../v2/live-run.schema.json"],',
        'const legacyEvidenceSchemaByVersion = new Map([\n  [1, "../v2/live-run.schema.json"],\n  [3, "../v3/run.schema.json"],',
      )
      .replace(
        "if (v3Kind === null) validateLiveRunSemantics(relativePath, value);",
        "if (false) validateLiveRunSemantics(relativePath, value);",
      );
    assert.notEqual(hostileChecker, checker);
    assert.match(hostileChecker, /\[3, "\.\.\/v3\/run\.schema\.json"\]/u);
    assert.match(hostileChecker, /if \(false\) validateLiveRunSemantics/u);
    await writeFile(checkerPath, hostileChecker);

    const evidencePath = fixture.addedReceipts[0].linkedEvidence.find(
      (link) => link.kind === "semantic",
    ).path;
    const evidence = JSON.parse(
      await readFile(path.join(fixture.repository, evidencePath), "utf8"),
    );
    evidence.checks[0].passed -= 1;
    await writeFile(path.join(fixture.repository, evidencePath), serialize(evidence));

    const output = await expectFailure(
      run(process.execPath, ["scripts/check-capabilities.mjs"], {
        cwd: fixture.repository,
      }),
    );
    assert.match(output, /passing V3 check is not all-pass/u);
  } finally {
    await fixture.cleanup();
  }
});

test("immutability protects the versioned V3 validator", async () => {
  const fixture = await createFixture();
  try {
    await appendFile(
      path.join(fixture.repository, "capabilities/v3/validator.mjs"),
      "\n// weakened validator fixture\n",
    );
    await commit(fixture.repository, "modify V3 validator");
    const output = await expectFailure(
      runImmutabilityCheck(fixture, fixture.implementationCommit),
    );
    assert.match(output, /capabilities\/v3\/validator\.mjs/u);
  } finally {
    await fixture.cleanup();
  }
});

for (const [label, relativePath, kind] of [
  [
    "symlink evidence",
    "capabilities/evidence/ignored.v3.json",
    "symlink",
  ],
  ["directory receipt", "capabilities/ignored.v3.json", "directory"],
]) {
  test(`capability discovery rejects ${label}`, async () => {
    const fixture = await createFixture();
    try {
      const absolutePath = path.join(fixture.repository, relativePath);
      if (kind === "symlink") {
        await symlink("badi-semantic-run.fixture.v3.json", absolutePath);
      } else {
        await mkdir(absolutePath);
      }
      const output = await expectFailure(
        run(process.execPath, ["scripts/check-capabilities.mjs"], {
          cwd: fixture.repository,
        }),
      );
      assert.match(output, /is not a regular file/u);
    } finally {
      await fixture.cleanup();
    }
  });
}

test("release mode rejects a candidate receipt", { timeout: 120_000 }, async () => {
  const fixture = await createFixture();
  try {
    const candidateId = fixture.addedReceipts[0].id;
    const output = await expectFailure(
      run(
        process.execPath,
        [
          "scripts/check-capabilities.mjs",
          "--require-current",
          "--receipt-id",
          candidateId,
          "--require-live",
        ],
        { cwd: fixture.repository },
      ),
    );
    assert.match(output, /selected release receipt is not a live V3 product cell/u);
  } finally {
    await fixture.cleanup();
  }
});

for (const brokerKind of ["symlink", "directory"]) {
  test(`strict-current rejects a ${brokerKind} repository artifact`, {
    timeout: 120_000,
  }, async () => {
    const fixture = await createFixture({ brokerKind });
    try {
      const output = await expectFailure(runAdditionsCheck(fixture));
      assert.match(
        output,
        brokerKind === "symlink"
          ? /current V3 artifact is not a regular file/u
          : /recorded V3 artifact .* is not a blob/u,
      );
    } finally {
      await fixture.cleanup();
    }
  });
}
