const LOCALHOST_TEXTAREA_TARGET =
  "http://localhost:4173/chromium.html :: HTMLTextAreaElement#draft";

export const PHRASE_V1_PROVIDER = "phrase_v1";
export const FIXTURE_FAULT_PROVIDER = "fixture_fault_v1";

export const PHRASE_V1_COMPLETIONS = Object.freeze({
  "thank you": " for your time",
  "looking forward": " to hearing from you",
  "the next step": " is to verify the result",
  please: " let me know what you think",
});

function providerCase(trigger, expectedOutput) {
  return Object.freeze({
    trigger,
    expected_output: expectedOutput,
  });
}

function phraseCase(trigger) {
  return providerCase(trigger, PHRASE_V1_COMPLETIONS[trigger]);
}

function phraseScenario(id, cases) {
  return Object.freeze({
    id,
    provider: PHRASE_V1_PROVIDER,
    evidence_class: "real-rust-chain",
    target: LOCALHOST_TEXTAREA_TARGET,
    cases: Object.freeze(cases),
  });
}

const allPhraseCases = () => Object.keys(PHRASE_V1_COMPLETIONS).map(phraseCase);

export const SCENARIO_PLAN = Object.freeze([
  phraseScenario("chromium.full-chain", [phraseCase("thank you")]),
  phraseScenario("interaction.dismiss", [phraseCase("looking forward")]),
  phraseScenario("interaction.undo", [phraseCase("the next step")]),
  phraseScenario("commit.accept-word", [phraseCase("please")]),
  phraseScenario("security.untrusted-keyboard", [phraseCase("thank you")]),
  phraseScenario("security.synthetic-focus-zero", [phraseCase("looking forward")]),
  phraseScenario("privacy.denied-zero", [
    phraseCase("thank you"),
    phraseCase("please"),
  ]),
  phraseScenario("lifecycle.dynamic-invalidation", [
    phraseCase("thank you"),
    phraseCase("looking forward"),
    phraseCase("the next step"),
  ]),
  phraseScenario("lifecycle.composition", [phraseCase("please")]),
  phraseScenario("geometry.scroll-zoom", [phraseCase("thank you")]),
  phraseScenario("lifecycle.visibility", [phraseCase("looking forward")]),
  phraseScenario("lifecycle.navigation", [
    phraseCase("the next step"),
    phraseCase("please"),
  ]),
  phraseScenario("control.pause-authoritative", [
    phraseCase("thank you"),
    phraseCase("looking forward"),
    phraseCase("the next step"),
  ]),
  phraseScenario("control.pause-shortcut", [
    phraseCase("please"),
    phraseCase("thank you"),
  ]),
  phraseScenario("schedule.debounce-latest", allPhraseCases()),
  phraseScenario("commit.insertion-100", allPhraseCases()),
  phraseScenario("latency.edit-to-visible", allPhraseCases()),
  phraseScenario("lifecycle.disconnect", [phraseCase("please")]),
  Object.freeze({
    id: "race.stale-100",
    provider: FIXTURE_FAULT_PROVIDER,
    evidence_class: "live-browser-fault-host",
    target: LOCALHOST_TEXTAREA_TARGET,
    cases: Object.freeze([
      providerCase("stale-live", " live"),
      providerCase("stale-live-final", " latest"),
    ]),
  }),
]);

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireNonemptyString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${label} must be a nonempty string`);
  }
}

export function validateScenarioPlan(plan = SCENARIO_PLAN) {
  if (!Array.isArray(plan) || plan.length === 0) {
    throw new Error("Scenario plan must be a nonempty array");
  }

  const ids = new Set();
  for (const [scenarioIndex, entry] of plan.entries()) {
    const label = `Scenario plan entry ${scenarioIndex}`;
    if (!isRecord(entry)) throw new Error(`${label} must be an object`);
    requireNonemptyString(entry.id, `${label} id`);
    requireNonemptyString(entry.provider, `${label} provider`);
    requireNonemptyString(entry.evidence_class, `${label} evidence_class`);
    requireNonemptyString(entry.target, `${label} target`);
    if (ids.has(entry.id)) throw new Error(`Duplicate scenario id: ${entry.id}`);
    ids.add(entry.id);

    const expectedEvidenceClass =
      entry.provider === PHRASE_V1_PROVIDER
        ? "real-rust-chain"
        : entry.provider === FIXTURE_FAULT_PROVIDER
          ? "live-browser-fault-host"
          : null;
    if (expectedEvidenceClass === null) {
      throw new Error(`${entry.id} declares unknown provider ${entry.provider}`);
    }
    if (entry.evidence_class !== expectedEvidenceClass) {
      throw new Error(`${entry.id} provider/evidence class mismatch`);
    }
    if (entry.target !== LOCALHOST_TEXTAREA_TARGET) {
      throw new Error(`${entry.id} declares an unapproved target`);
    }
    if (!Array.isArray(entry.cases) || entry.cases.length === 0) {
      throw new Error(`${entry.id} must declare at least one provider case`);
    }

    for (const [caseIndex, testCase] of entry.cases.entries()) {
      const caseLabel = `${entry.id} case ${caseIndex}`;
      if (!isRecord(testCase)) throw new Error(`${caseLabel} must be an object`);
      requireNonemptyString(testCase.trigger, `${caseLabel} trigger`);
      requireNonemptyString(testCase.expected_output, `${caseLabel} expected_output`);
      if (entry.provider !== PHRASE_V1_PROVIDER) continue;

      const expected = PHRASE_V1_COMPLETIONS[testCase.trigger];
      if (expected === undefined) {
        throw new Error(
          `${entry.id} uses unsupported phrase_v1 trigger ${testCase.trigger}`,
        );
      }
      if (testCase.expected_output !== expected) {
        throw new Error(`${entry.id} phrase_v1 expected output mismatch`);
      }
    }
  }
  return plan;
}

export function scenarioDefinition(id) {
  const entry = SCENARIO_PLAN.find((candidate) => candidate.id === id);
  if (entry === undefined) throw new Error(`Scenario is absent from plan: ${id}`);
  return entry;
}

export function scenarioCase(id, index = 0) {
  const entry = scenarioDefinition(id);
  const testCase = entry.cases[index];
  if (testCase === undefined) {
    throw new Error(`Scenario case is absent from plan: ${id}[${index}]`);
  }
  return testCase;
}

export function validateCompletedScenarioIds(ids) {
  const expected = SCENARIO_PLAN.map((entry) => entry.id).sort();
  const actual = [...ids].sort();
  if (
    expected.length !== actual.length ||
    expected.some((id, index) => id !== actual[index])
  ) {
    throw new Error("Completed scenarios do not exactly match the scenario plan");
  }
}

validateScenarioPlan();
