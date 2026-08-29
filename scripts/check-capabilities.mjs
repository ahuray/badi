import { access, readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import process from "node:process";
import Ajv2020 from "ajv/dist/2020.js";

const repository = path.resolve(import.meta.dirname, "..");
const capabilityRoot = path.join(repository, "capabilities");
const schema = JSON.parse(
  await readFile(path.join(capabilityRoot, "v1/schema.json"), "utf8"),
);
const validate = new Ajv2020({ allErrors: true, strict: true }).compile(schema);
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
    }),
  );
}

for (const file of files) {
  const value = JSON.parse(
    await readFile(path.join(capabilityRoot, file), "utf8"),
  );
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
