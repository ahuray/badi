import { access, readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import process from "node:process";
import { isDeepStrictEqual } from "node:util";
import Ajv2020 from "ajv/dist/2020.js";
import { assertExactChromiumManifest } from "../adapters/chromium/scripts/manifest-policy.mjs";

const repository = path.resolve(import.meta.dirname, "..");
const capabilityRoot = path.join(repository, "capabilities");
const validators = new Map();
const files = (await readdir(capabilityRoot, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
  .map((entry) => entry.name)
  .sort();

let failed = false;
if (files.length === 0) {
  failed = true;
  process.stderr.write("No capability receipts found.\n");
}

function resolveRepositoryPath(relativePath, label) {
  const resolved = path.resolve(repository, relativePath);
  if (
    path.isAbsolute(relativePath) ||
    (resolved !== repository && !resolved.startsWith(`${repository}${path.sep}`))
  ) {
    throw new Error(`${label}: path escapes the repository: ${relativePath}`);
  }
  return resolved;
}

async function validateEvidenceLinks(file, value) {
  const protocolSchema = resolveRepositoryPath(
    value.protocol.schema,
    `${file} protocol.schema`,
  );
  const adapterManifest = resolveRepositoryPath(
    value.adapter.manifest,
    `${file} adapter.manifest`,
  );
  const buildManifestPath = resolveRepositoryPath(
    value.adapter.build_manifest,
    `${file} adapter.build_manifest`,
  );
  await Promise.all([
    access(protocolSchema),
    access(adapterManifest),
    access(buildManifestPath),
  ]);

  const extensionManifest = JSON.parse(await readFile(adapterManifest, "utf8"));
  assertExactChromiumManifest(extensionManifest);

  const buildManifest = JSON.parse(await readFile(buildManifestPath, "utf8"));
  if (
    buildManifest.package !== value.adapter.package ||
    buildManifest.version !== value.adapter.version ||
    buildManifest.target !== value.adapter.target ||
    buildManifest.native_host !== value.adapter.native_host ||
    !Array.isArray(buildManifest.artifacts)
  ) {
    throw new Error(`${file}: adapter metadata differs from its build manifest`);
  }

  const artifactDirectory = path.posix.dirname(value.adapter.build_manifest);
  const normalize = (artifact) => ({
    path: artifact.path,
    bytes: artifact.bytes,
    sha256: artifact.sha256,
  });
  const expected = [...(value.artifacts ?? [])].map(normalize).sort((a, b) =>
    a.path.localeCompare(b.path),
  );
  const actual = buildManifest.artifacts
    .map((artifact) =>
      normalize({
        ...artifact,
        path: path.posix.join(artifactDirectory, artifact.path),
      }),
    )
    .sort((a, b) => a.path.localeCompare(b.path));
  if (JSON.stringify(expected) !== JSON.stringify(actual)) {
    throw new Error(
      `${file}: artifact hashes differ from the generated build manifest`,
    );
  }

  await Promise.all(
    expected.map(async (artifact) => {
      const artifactPath = resolveRepositoryPath(
        artifact.path,
        `${file} artifact`,
      );
      const bytes = await readFile(artifactPath);
      const digest = createHash("sha256").update(bytes).digest("hex");
      if (bytes.byteLength !== artifact.bytes || digest !== artifact.sha256) {
        throw new Error(`${file}: generated artifact differs: ${artifact.path}`);
      }
      if (artifact.path === path.posix.join(artifactDirectory, "manifest.json")) {
        assertExactChromiumManifest(JSON.parse(bytes.toString("utf8")));
      }
    }),
  );

  if (value.record_version >= 2) {
    requireUniqueValues(
      file,
      value.evidence.checks.map((check) => check.id),
      "check id",
    );
    requireUniqueValues(
      file,
      value.evidence.measurements.map((measurement) => measurement.name),
      "measurement name",
    );
    const nativeSource = resolveRepositoryPath(
      value.native_host.source,
      `${file} native_host.source`,
    );
    const nativeManifest = resolveRepositoryPath(
      value.native_host.manifest_example,
      `${file} native_host.manifest_example`,
    );
    await Promise.all([access(nativeSource), access(nativeManifest)]);
    const decodedKey = Buffer.from(extensionManifest.key ?? "", "base64");
    const keyDigest = createHash("sha256").update(decodedKey).digest();
    const derivedExtensionId = [...keyDigest.subarray(0, 16)]
      .flatMap((byte) => [byte >>> 4, byte & 0x0f])
      .map((nibble) => String.fromCharCode("a".charCodeAt(0) + nibble))
      .join("");
    if (derivedExtensionId !== value.adapter.extension_id) {
      throw new Error(`${file}: manifest key differs from receipt extension id`);
    }
    if (
      JSON.stringify(extensionManifest.permissions) !== JSON.stringify(["nativeMessaging"]) ||
      extensionManifest.incognito !== "not_allowed" ||
      extensionManifest.minimum_chrome_version !== "132"
    ) {
      throw new Error(`${file}: live extension permissions/runtime boundary is not exact`);
    }
    const nativeManifestValue = JSON.parse(await readFile(nativeManifest, "utf8"));
    const expectedOrigin = `chrome-extension://${value.adapter.extension_id}/`;
    if (
      nativeManifestValue.name !== value.adapter.native_host ||
      nativeManifestValue.type !== "stdio" ||
      !path.isAbsolute(nativeManifestValue.path ?? "") ||
      JSON.stringify(nativeManifestValue.allowed_origins) !==
        JSON.stringify([expectedOrigin])
    ) {
      throw new Error(`${file}: native manifest example differs from exact live identity`);
    }
    const validatedDocuments = await validateHashedArtifacts(
      file,
      value.evidence_artifacts,
      "evidence_artifact",
    );
    const liveRuns = validatedDocuments.filter(
      (document) => document.value.id === "chromium-native-live-run.v1",
    );
    if (liveRuns.length !== 1) {
      throw new Error(`${file}: expected exactly one linked Chromium live-run document`);
    }
    await validateLiveReceiptLink(file, value, liveRuns[0].value);
  }
}

function requireUniqueValues(file, values, label) {
  const unique = new Set(values);
  if (unique.size !== values.length) {
    throw new Error(`${file}: duplicate ${label}`);
  }
}

async function validateHashedArtifacts(file, artifacts, label) {
  const paths = new Set();
  const validatedDocuments = [];
  for (const artifact of artifacts) {
    if (paths.has(artifact.path)) {
      throw new Error(`${file}: duplicate ${label} path: ${artifact.path}`);
    }
    paths.add(artifact.path);
    const artifactPath = resolveRepositoryPath(
      artifact.path,
      `${file} ${label}`,
    );
    const bytes = await readFile(artifactPath);
    const digest = createHash("sha256").update(bytes).digest("hex");
    if (bytes.byteLength !== artifact.bytes || digest !== artifact.sha256) {
      throw new Error(`${file}: ${label} differs: ${artifact.path}`);
    }
    if (artifact.path.endsWith(".json")) {
      const value = JSON.parse(bytes.toString("utf8"));
      const validate = await validatorForDocument(artifactPath, artifact.path, value);
      if (!validate(value)) {
        throw new Error(
          `${artifact.path}: ${JSON.stringify(validate.errors, null, 2)}`,
        );
      }
      validateLiveRunSemantics(artifact.path, value);
      validatedDocuments.push({ path: artifact.path, value });
    }
  }
  return validatedDocuments;
}

async function validateLiveReceiptLink(file, receipt, run) {
  const receiptTime = Date.parse(receipt.recorded_at);
  const runTime = Date.parse(run.recorded_at);
  if (!Number.isFinite(receiptTime) || !Number.isFinite(runTime) || receiptTime < runTime) {
    throw new Error(`${file}: receipt predates its linked live run`);
  }

  if (!isDeepStrictEqual(receipt.evidence.repository, run.repository)) {
    throw new Error(`${file}: repository provenance differs from linked live run`);
  }
  if (!isDeepStrictEqual(receipt.evidence.local_environment, run.environment)) {
    throw new Error(`${file}: local environment differs from linked live run`);
  }
  if (!isDeepStrictEqual(receipt.evidence.privacy, run.privacy)) {
    throw new Error(`${file}: privacy claims differ from linked live run`);
  }

  for (const [receiptKey, runKey] of [
    ["extension_id", "id"],
    ["minimum_chrome_version", "minimum_chrome_version"],
    ["permission_mode", "permission_mode"],
    ["incognito", "incognito"],
    ["host_match", "host_match"],
  ]) {
    if (receipt.adapter[receiptKey] !== run.extension[runKey]) {
      throw new Error(`${file}: adapter differs from linked live run: ${receiptKey}`);
    }
  }
  if (!isDeepStrictEqual(receipt.adapter.api_permissions, run.extension.api_permissions)) {
    throw new Error(`${file}: adapter API permissions differ from linked live run`);
  }
  if (receipt.adapter.native_host !== run.native.host_name) {
    throw new Error(`${file}: native host name differs from linked live run`);
  }
  for (const key of [
    "caller_origin",
    "max_envelope_bytes",
    "socket_parent_mode",
    "socket_mode",
    "peer_uid_verified",
  ]) {
    if (receipt.native_host[key] !== run.native[key]) {
      throw new Error(`${file}: native host boundary differs from linked live run: ${key}`);
    }
  }

  const checks = new Map(receipt.evidence.checks.map((check) => [check.id, check]));
  const scenarios = new Map(run.scenarios.map((scenario) => [scenario.id, scenario]));
  for (const id of [
    "chromium.full-chain",
    "interaction.dismiss",
    "commit.accept-word",
    "control.pause-authoritative",
    "privacy.denied-zero",
    "race.stale-100",
    "commit.insertion-100",
    "security.untrusted-keyboard",
    "security.synthetic-focus-zero",
    "lifecycle.dynamic-invalidation",
    "lifecycle.composition",
    "geometry.scroll-zoom",
    "lifecycle.navigation",
    "lifecycle.disconnect",
  ]) {
    const check = checks.get(id);
    const scenario = scenarios.get(id);
    if (check === undefined || scenario === undefined || scenario.status !== "pass") {
      throw new Error(`${file}: linked live scenario missing or not passing: ${id}`);
    }
    if (check.trials !== scenario.trials || check.passed !== scenario.passed) {
      throw new Error(`${file}: check counts differ from linked live scenario: ${id}`);
    }
  }

  const commands = new Map(run.commands.map((command) => [command.id, command]));
  for (const id of [
    "cargo-build-bins",
    "cargo-test-workspace",
    "chromium-typecheck",
    "chromium-unit",
    "chromium-build-verify",
    "isolated-live-run",
  ]) {
    if (commands.get(id)?.exit_code !== 0) {
      throw new Error(`${file}: linked provenance command missing or failed: ${id}`);
    }
  }
  const rustCheck = checks.get("rust.native-host");
  if (
    rustCheck === undefined ||
    commands.get("cargo-build-bins")?.exit_code !== 0 ||
    commands.get("cargo-test-workspace")?.exit_code !== 0
  ) {
    throw new Error(`${file}: Rust native-host check lacks linked build/test provenance`);
  }
  const repositoryCheck = checks.get("repository.validation");
  if (
    repositoryCheck?.trials !== run.commands.length ||
    repositoryCheck.passed !== run.commands.length
  ) {
    throw new Error(`${file}: repository validation count differs from linked commands`);
  }
  const cleanupCheck = checks.get("isolation.cleanup");
  if (
    cleanupCheck?.trials !== 3 ||
    cleanupCheck.passed !== 3 ||
    run.isolation.temporary_tree_removed !== true ||
    run.isolation.socket_removed !== true ||
    run.isolation.processes_remaining !== 0
  ) {
    throw new Error(`${file}: cleanup check differs from linked live run`);
  }

  const measurements = new Map(
    receipt.evidence.measurements.map((measurement) => [measurement.name, measurement]),
  );
  const runMeasurements = new Map(
    run.measurements.map((measurement) => [measurement.name, measurement]),
  );
  for (const id of ["invalidation-to-hide", "accept-to-insert"]) {
    const measurement = measurements.get(id);
    const runMeasurement = runMeasurements.get(id);
    if (measurement === undefined || runMeasurement === undefined) {
      throw new Error(`${file}: linked measurement missing: ${id}`);
    }
    for (const [receiptKey, runKey] of [
      ["samples", "samples"],
      ["warmups", "warmups"],
      ["evidence_class", "evidence_class"],
      ["unit", "unit"],
      ["statistic", "statistic"],
      ["value", "p95"],
      ["maximum", "maximum"],
      ["threshold", "threshold"],
      ["status", "status"],
    ]) {
      if (measurement[receiptKey] !== runMeasurement[runKey]) {
        throw new Error(`${file}: measurement differs from linked run: ${id}.${receiptKey}`);
      }
    }
  }

  const runArtifacts = new Map(run.artifacts.map((artifact) => [artifact.id, artifact]));
  const extensionArtifactIds = new Map([
    ["content-script.js", "extension-content-script"],
    ["manifest.json", "extension-manifest"],
    ["service-worker.js", "extension-service-worker"],
  ]);
  for (const artifact of receipt.artifacts) {
    const fileName = path.posix.basename(artifact.path);
    const runId = extensionArtifactIds.get(fileName);
    const runArtifact = runId === undefined ? undefined : runArtifacts.get(runId);
    if (
      runArtifact === undefined ||
      runArtifact.bytes !== artifact.bytes ||
      runArtifact.sha256 !== artifact.sha256
    ) {
      throw new Error(`${file}: extension artifact differs from linked run: ${fileName}`);
    }
  }

  const repositoryRunArtifacts = new Map([
    ["extension-build-manifest", "adapters/chromium/dist/BUILD_MANIFEST.json"],
    ["fixture-html", "fixtures/web/chromium.html"],
    ["fixture-js", "fixtures/web/fixture.js"],
    ["fixture-css", "fixtures/web/fixture.css"],
    ["live-runner", "adapters/chromium/live/run-live.mjs"],
    ["fault-host", "adapters/chromium/live/fake-native-host.mjs"],
    ["manifest-policy", "adapters/chromium/scripts/manifest-policy.mjs"],
  ]);
  for (const [id, relativePath] of repositoryRunArtifacts) {
    const artifact = runArtifacts.get(id);
    if (artifact === undefined) {
      throw new Error(`${file}: linked live artifact missing: ${id}`);
    }
    const artifactPath = resolveRepositoryPath(relativePath, `${file} live artifact`);
    const bytes = await readFile(artifactPath);
    const digest = createHash("sha256").update(bytes).digest("hex");
    if (artifact.bytes !== bytes.byteLength || artifact.sha256 !== digest) {
      throw new Error(`${file}: linked live artifact differs: ${id}`);
    }
  }

  if (!isDeepStrictEqual(receipt.evidence.isolation, run.isolation)) {
    throw new Error(`${file}: isolation claims differ from linked live run`);
  }
}

function validateLiveRunSemantics(file, value) {
  if (value.id !== "chromium-native-live-run.v1") return;
  requireUniqueValues(file, value.commands.map((command) => command.id), "command id");
  requireUniqueValues(file, value.scenarios.map((scenario) => scenario.id), "scenario id");
  requireUniqueValues(
    file,
    value.measurements.map((measurement) => measurement.name),
    "measurement name",
  );
  requireUniqueValues(file, value.artifacts.map((artifact) => artifact.id), "artifact id");
  for (const scenario of value.scenarios) {
    if (scenario.passed > scenario.trials) {
      throw new Error(`${file}: scenario passed count exceeds trials: ${scenario.id}`);
    }
    if (scenario.status === "pass" && scenario.passed !== scenario.trials) {
      throw new Error(`${file}: passing scenario is not all-pass: ${scenario.id}`);
    }
  }
  const serialized = JSON.stringify(value);
  if (/\/(?:home|Users)\//u.test(serialized)) {
    throw new Error(`${file}: evidence contains an absolute personal path`);
  }
}

async function validatorForDocument(documentPath, file, value) {
  if (typeof value.$schema !== "string") {
    throw new Error(`${file}: missing $schema`);
  }
  if (path.isAbsolute(value.$schema)) {
    throw new Error(`${file}: absolute schema path is forbidden`);
  }
  const schemaPath = path.resolve(path.dirname(documentPath), value.$schema);
  if (
    schemaPath !== capabilityRoot &&
    !schemaPath.startsWith(`${capabilityRoot}${path.sep}`)
  ) {
    throw new Error(`${file}: schema is outside capabilities/`);
  }
  let validate = validators.get(schemaPath);
  if (validate === undefined) {
    const schema = JSON.parse(await readFile(schemaPath, "utf8"));
    validate = new Ajv2020({ allErrors: true, strict: true }).compile(schema);
    validators.set(schemaPath, validate);
  }
  return validate;
}

for (const file of files) {
  const documentPath = path.join(capabilityRoot, file);
  const value = JSON.parse(await readFile(documentPath, "utf8"));
  let validate;
  try {
    validate = await validatorForDocument(documentPath, file, value);
  } catch (error) {
    failed = true;
    const detail = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${detail}\n`);
    continue;
  }
  if (!validate(value)) {
    failed = true;
    process.stderr.write(
      `${file}: ${JSON.stringify(validate.errors, null, 2)}\n`,
    );
    continue;
  }
  try {
    await validateEvidenceLinks(file, value);
  } catch (error) {
    failed = true;
    const detail = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${detail}\n`);
  }
}

if (failed) {
  process.exitCode = 1;
} else {
  process.stdout.write(`Validated ${files.length} capability receipt(s).\n`);
}
