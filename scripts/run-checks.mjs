// Run every repository check named below (or on the command line), continue
// past failures, and finish with one summary. Exits non-zero if any failed.
// Checks run cheapest first so quick failures surface early.
import { spawnSync } from "node:child_process";
import process from "node:process";

const DEFAULT_CHECKS = [
  "docs:check",
  "accessibility:check",
  "desktop:check",
  "omarchy:check",
  "editors:check",
  "writing:check",
];

const checks = process.argv.length > 2 ? process.argv.slice(2) : DEFAULT_CHECKS;
const results = [];

for (const name of checks) {
  process.stdout.write(`\n=== ${name} ===\n`);
  const started = process.hrtime.bigint();
  const run = spawnSync("npm", ["run", name], { stdio: "inherit" });
  const seconds = Number(process.hrtime.bigint() - started) / 1e9;
  if (run.signal === "SIGINT" || run.signal === "SIGTERM") {
    // An interrupted run is not a result; stop instead of starting the next check.
    process.stderr.write(`\n${name} interrupted by ${run.signal}; remaining checks skipped.\n`);
    process.exit(run.signal === "SIGINT" ? 130 : 143);
  }
  const failure = run.error?.message ?? (run.status === 0 ? null : `exit ${run.status ?? run.signal}`);
  results.push({ name, seconds, failure });
}

process.stdout.write("\n=== Summary ===\n");
for (const { name, seconds, failure } of results) {
  process.stdout.write(`${failure === null ? "pass" : "FAIL"}  ${name} (${seconds.toFixed(1)} s)${failure === null ? "" : ` — ${failure}`}\n`);
}
const failed = results.filter((result) => result.failure !== null);
if (failed.length > 0) {
  process.stderr.write(`${failed.length} of ${results.length} checks failed: ${failed.map((result) => result.name).join(", ")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`All ${results.length} checks passed.\n`);
}
