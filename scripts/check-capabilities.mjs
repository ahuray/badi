import { access, readFile, readdir } from "node:fs/promises";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import path from "node:path";
import process from "node:process";
import { isDeepStrictEqual, promisify } from "node:util";
import Ajv2020 from "ajv/dist/2020.js";
import { assertExactChromiumManifest } from "../adapters/chromium/scripts/manifest-policy.mjs";

const repository = path.resolve(import.meta.dirname, "..");
const capabilityRoot = path.join(repository, "capabilities");
const execFileAsync = promisify(execFile);
const maximumGitBlobBytes = 4 * 1024 * 1024;
const liveRunIdPattern =
  /^chromium-native-live-run(?:\.[a-z0-9][a-z0-9-]{0,63})?\.v1$/u;
const cliArguments = process.argv.slice(2);
if (cliArguments.includes("--help")) {
  process.stdout.write(
      "Usage: node scripts/check-capabilities.mjs [--require-current]\n" +
      "Without the flag, V2 evidence is validated at its recorded Git commit.\n" +
      "--require-current also requires current adapter artifacts and the complete Rust source/input set to match the recorded clean commit.\n" +
      "Build the extension first when invoking this file directly.\n",
  );
  process.exit(0);
}
if (
  cliArguments.length > 1 ||
  (cliArguments.length === 1 && cliArguments[0] !== "--require-current")
) {
  process.stderr.write(
    "Usage: node scripts/check-capabilities.mjs [--require-current]\n",
  );
  process.exit(2);
}
const requireCurrent = cliArguments[0] === "--require-current";
const validators = new Map();
const files = (await readdir(capabilityRoot, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
  .map((entry) => entry.name)
  .sort();

let failed = false;
let anchoredV2 = 0;
let unanchoredV1 = 0;
let currentV2FullSource = 0;
let currentV1Adapter = 0;
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

function validateDeclaredPaths(file, value) {
  resolveRepositoryPath(value.protocol.schema, `${file} protocol.schema`);
  resolveRepositoryPath(value.adapter.manifest, `${file} adapter.manifest`);
  resolveRepositoryPath(
    value.adapter.build_manifest,
    `${file} adapter.build_manifest`,
  );
  requireUniqueValues(
    file,
    value.artifacts.map((artifact) => artifact.path),
    "artifact path",
  );
  for (const artifact of value.artifacts) {
    resolveRepositoryPath(artifact.path, `${file} artifact`);
  }
  if (value.record_version >= 2) {
    resolveRepositoryPath(value.native_host.source, `${file} native_host.source`);
    resolveRepositoryPath(
      value.native_host.manifest_example,
      `${file} native_host.manifest_example`,
    );
    for (const artifact of value.evidence_artifacts) {
      resolveRepositoryPath(artifact.path, `${file} evidence_artifact`);
    }
  }
}

async function runGit(gitArguments, label, options = {}) {
  try {
    const result = await execFileAsync("git", gitArguments, {
      cwd: repository,
      encoding: "utf8",
      maxBuffer: 64 * 1024,
      timeout: 5_000,
      ...options,
    });
    return result.stdout;
  } catch {
    throw new Error(`${label}: recorded Git object is unavailable`);
  }
}

async function requireRecordedCommit(file, commit) {
  if (!/^[0-9a-f]{40}$/u.test(commit)) {
    throw new Error(`${file}: recorded base commit is not a full Git object id`);
  }
  await runGit(
    ["cat-file", "-e", `${commit}^{commit}`],
    `${file} base commit ${commit}`,
  );
}

async function readRecordedBlob(commit, relativePath, label) {
  resolveRepositoryPath(relativePath, label);
  const object = `${commit}:${relativePath}`;
  const type = await runGit(["cat-file", "-t", object], label);
  if (type.trim() !== "blob") {
    throw new Error(`${label}: recorded Git object is not a blob`);
  }
  const sizeText = await runGit(["cat-file", "-s", object], label);
  const size = Number(sizeText.trim());
  if (
    !Number.isSafeInteger(size) ||
    size < 0 ||
    size > maximumGitBlobBytes
  ) {
    throw new Error(`${label}: recorded Git blob has an unsupported size`);
  }
  const bytes = await runGit(["cat-file", "blob", object], label, {
    encoding: null,
    maxBuffer: size + 1024,
  });
  if (!Buffer.isBuffer(bytes) || bytes.byteLength !== size) {
    throw new Error(`${label}: recorded Git blob size changed while reading`);
  }
  return bytes;
}

async function currentRustChainInputs() {
  const inputs = ["Cargo.toml", "Cargo.lock"];
  async function collect(relativeDirectory) {
    const directory = resolveRepositoryPath(
      relativeDirectory,
      `current Rust input directory ${relativeDirectory}`,
    );
    const entries = await readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
      const relativePath = path.posix.join(relativeDirectory, entry.name);
      if (entry.isDirectory()) {
        await collect(relativePath);
      } else if (entry.isFile()) {
        inputs.push(relativePath);
      }
    }
  }
  await collect("broker");
  await collect("protocol/v1");
  return inputs.sort();
}

async function recordedRustChainInputs(file, commit) {
  const sourceList = await runGit(
    [
      "ls-tree",
      "-r",
      "--name-only",
      commit,
      "--",
      "broker",
      "protocol/v1",
    ],
    `${file} recorded Rust input list`,
  );
  return [
    "Cargo.toml",
    "Cargo.lock",
    ...sourceList
      .split("\n")
      .filter((relativePath) => relativePath.length > 0),
  ].sort();
}

async function validateCurrentRustChainInputs(file, liveRun) {
  const commit = liveRun.repository.base_commit;
  const [currentInputs, recordedInputs] = await Promise.all([
    currentRustChainInputs(),
    recordedRustChainInputs(file, commit),
  ]);
  if (!isDeepStrictEqual(currentInputs, recordedInputs)) {
    throw new Error(
      `${file}: current Rust-chain source/build/test input set differs from recorded commit ${commit}`,
    );
  }
  await Promise.all(
    currentInputs.map(async (relativePath) => {
      const [currentBytes, recordedBytes] = await Promise.all([
        readFile(
          resolveRepositoryPath(relativePath, `${file} current Rust-chain input`),
        ),
        readRecordedBlob(
          commit,
          relativePath,
          `${file} recorded Rust-chain input ${relativePath}`,
        ),
      ]);
      if (!currentBytes.equals(recordedBytes)) {
        throw new Error(
          `${file}: current Rust-chain input differs from recorded commit: ${relativePath}`,
        );
      }
    }),
  );
}

async function validateCurrentRecordedPaths(file, commit, relativePaths) {
  await Promise.all(
    relativePaths.map(async (relativePath) => {
      const [currentBytes, recordedBytes] = await Promise.all([
        readFile(
          resolveRepositoryPath(relativePath, `${file} current declared input`),
        ),
        readRecordedBlob(
          commit,
          relativePath,
          `${file} recorded declared input ${relativePath}`,
        ),
      ]);
      if (!currentBytes.equals(recordedBytes)) {
        throw new Error(
          `${file}: current declared input differs from recorded commit: ${relativePath}`,
        );
      }
    }),
  );
}

async function validateCurrentLinks(file, value, liveRun) {
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

  // Check the complete native source/build/test input set before generated
  // adapter artifacts. The validator is fail-fast per receipt, so this order
  // ensures a changed Rust chain cannot be hidden behind the first extension
  // hash mismatch.
  if (value.record_version >= 2) {
    await validateCurrentRustChainInputs(file, liveRun);
  }

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
    const nativeSource = resolveRepositoryPath(
      value.native_host.source,
      `${file} native_host.source`,
    );
    const nativeManifest = resolveRepositoryPath(
      value.native_host.manifest_example,
      `${file} native_host.manifest_example`,
    );
    await Promise.all([access(nativeSource), access(nativeManifest)]);
    const nativeManifestValue = JSON.parse(await readFile(nativeManifest, "utf8"));
    validateLiveBoundary(
      file,
      value,
      extensionManifest,
      nativeManifestValue,
    );
    await validateCurrentRecordedPaths(
      file,
      liveRun.repository.base_commit,
      [
        value.protocol.schema,
        value.native_host.source,
        value.native_host.manifest_example,
      ],
    );
    await validateLinkedRepositoryArtifacts(
      file,
      liveRun,
      async (relativePath, label) =>
        readFile(resolveRepositoryPath(relativePath, label)),
      true,
      "current checkout",
    );
  }
}

function validateLiveBoundary(file, value, extensionManifest, nativeManifestValue) {
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
    JSON.stringify(extensionManifest.permissions) !==
      JSON.stringify(["nativeMessaging"]) ||
    extensionManifest.incognito !== "not_allowed" ||
    extensionManifest.minimum_chrome_version !== "132"
  ) {
    throw new Error(
      `${file}: live extension permissions/runtime boundary is not exact`,
    );
  }
  const expectedOrigin = `chrome-extension://${value.adapter.extension_id}/`;
  if (
    nativeManifestValue.name !== value.adapter.native_host ||
    nativeManifestValue.type !== "stdio" ||
    !path.isAbsolute(nativeManifestValue.path ?? "") ||
    JSON.stringify(nativeManifestValue.allowed_origins) !==
      JSON.stringify([expectedOrigin])
  ) {
    throw new Error(
      `${file}: native manifest example differs from exact live identity`,
    );
  }
}

async function validateHistoricalV2(file, value) {
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
  const validatedDocuments = await validateHashedArtifacts(
    file,
    value.evidence_artifacts,
    "evidence_artifact",
  );
  const liveRuns = validatedDocuments.filter(
    (document) => liveRunIdPattern.test(document.value.id),
  );
  if (liveRuns.length !== 1) {
    throw new Error(`${file}: expected exactly one linked Chromium live-run document`);
  }
  const liveRun = liveRuns[0].value;
  const expectedLiveRunPath = `capabilities/evidence/${liveRun.id}.json`;
  if (liveRuns[0].path !== expectedLiveRunPath) {
    throw new Error(
      `${file}: live-run id does not match its evidence path: ${liveRuns[0].path}`,
    );
  }
  await validateLiveReceiptLink(file, value, liveRun);

  const commit = value.evidence.repository.base_commit;
  await requireRecordedCommit(file, commit);
  const [, extensionManifestBytes, , nativeManifestBytes] = await Promise.all([
    readRecordedBlob(
      commit,
      value.protocol.schema,
      `${file} recorded protocol schema`,
    ),
    readRecordedBlob(
      commit,
      value.adapter.manifest,
      `${file} recorded adapter manifest`,
    ),
    readRecordedBlob(
      commit,
      value.native_host.source,
      `${file} recorded native-host source`,
    ),
    readRecordedBlob(
      commit,
      value.native_host.manifest_example,
      `${file} recorded native-host manifest`,
    ),
  ]);
  const extensionManifest = JSON.parse(extensionManifestBytes.toString("utf8"));
  const nativeManifest = JSON.parse(nativeManifestBytes.toString("utf8"));
  assertExactChromiumManifest(extensionManifest);
  validateLiveBoundary(file, value, extensionManifest, nativeManifest);
  const manifestArtifacts = value.artifacts.filter(
    (artifact) => path.posix.basename(artifact.path) === "manifest.json",
  );
  const manifestDigest = createHash("sha256")
    .update(extensionManifestBytes)
    .digest("hex");
  if (
    manifestArtifacts.length !== 1 ||
    manifestArtifacts[0].bytes !== extensionManifestBytes.byteLength ||
    manifestArtifacts[0].sha256 !== manifestDigest
  ) {
    throw new Error(
      `${file}: extension manifest artifact differs at recorded commit ${commit}`,
    );
  }
  await validateLinkedRepositoryArtifacts(
    file,
    liveRun,
    (relativePath, label) =>
      readRecordedBlob(commit, relativePath, label),
    false,
    `recorded commit ${commit}`,
  );
  return liveRun;
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
  if (
    receipt.evidence.repository.working_tree_dirty !== false ||
    run.repository.working_tree_dirty !== false
  ) {
    throw new Error(`${file}: durable evidence was not recorded from a clean tree`);
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

  if (!isDeepStrictEqual(receipt.evidence.isolation, run.isolation)) {
    throw new Error(`${file}: isolation claims differ from linked live run`);
  }
}

async function validateLinkedRepositoryArtifacts(
  file,
  run,
  readArtifact,
  includeGeneratedBuildManifest,
  location,
) {
  const artifacts = [
    ["fixture-html", "fixtures/web/chromium.html"],
    ["fixture-js", "fixtures/web/fixture.js"],
    ["fixture-css", "fixtures/web/fixture.css"],
    ["live-runner", "adapters/chromium/live/run-live.mjs"],
    ["fault-host", "adapters/chromium/live/fake-native-host.mjs"],
    ["manifest-policy", "adapters/chromium/scripts/manifest-policy.mjs"],
  ];
  if (includeGeneratedBuildManifest) {
    artifacts.unshift([
      "extension-build-manifest",
      "adapters/chromium/dist/BUILD_MANIFEST.json",
    ]);
  }
  const runArtifacts = new Map(
    run.artifacts.map((artifact) => [artifact.id, artifact]),
  );
  for (const [id, relativePath] of artifacts) {
    const artifact = runArtifacts.get(id);
    if (artifact === undefined) {
      throw new Error(`${file}: linked live artifact missing: ${id}`);
    }
    const bytes = await readArtifact(
      relativePath,
      `${file} ${location} artifact ${id}`,
    );
    const digest = createHash("sha256").update(bytes).digest("hex");
    if (artifact.bytes !== bytes.byteLength || artifact.sha256 !== digest) {
      throw new Error(`${file}: linked live artifact differs at ${location}: ${id}`);
    }
  }
}

function validateLiveRunSemantics(file, value) {
  if (!liveRunIdPattern.test(value.id)) return;
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
    validateDeclaredPaths(file, value);
    let liveRun;
    if (value.record_version >= 2) {
      liveRun = await validateHistoricalV2(file, value);
    }
    if (requireCurrent) {
      await validateCurrentLinks(file, value, liveRun);
    }
    if (value.record_version >= 2) {
      anchoredV2 += 1;
    } else {
      unanchoredV1 += 1;
    }
    if (requireCurrent) {
      if (value.record_version >= 2) {
        currentV2FullSource += 1;
      } else {
        currentV1Adapter += 1;
      }
    }
  } catch (error) {
    failed = true;
    const detail = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${detail}\n`);
  }
}

if (failed) {
  process.exitCode = 1;
} else {
  const mode = requireCurrent ? "strict-current" : "historical";
  const currentStatus = requireCurrent
    ? `v2_full_source_current=${currentV2FullSource}, v1_adapter_current=${currentV1Adapter}`
    : "current_links=not-checked";
  process.stdout.write(
    `Validated ${files.length} capability receipt(s) ` +
      `(mode=${mode}, v2_recorded_commit=${anchoredV2}, ` +
      `v1_unanchored=${unanchoredV1}, ${currentStatus}).\n`,
  );
  if (!requireCurrent) {
    process.stdout.write(
      "Current linked sources and generated artifacts were not checked; " +
        "use --require-current for that gate.\n",
    );
  }
  if (unanchoredV1 > 0) {
    process.stdout.write(
      "V1 receipts have no recorded commit or raw run; their historical " +
        "validation covers only schema and safe declared paths.\n",
    );
  }
}
