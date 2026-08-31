import { describe, expect, it } from "vitest";
import {
  PHRASE_V1_COMPLETIONS,
  SCENARIO_PLAN,
  scenarioDefinition,
  validateCompletedScenarioIds,
  validateScenarioPlan,
} from "../live/scenario-plan.mjs";

describe("live Chromium scenario plan", () => {
  it("declares a valid provider, evidence class, target, trigger, and output", () => {
    expect(() => validateScenarioPlan()).not.toThrow();
    expect(SCENARIO_PLAN.every((entry) => entry.cases.length > 0)).toBe(true);
    expect(
      SCENARIO_PLAN.every(
        (entry) =>
          entry.provider.length > 0 &&
          entry.evidence_class.length > 0 &&
          entry.target.length > 0,
      ),
    ).toBe(true);
  });

  it("limits phrase_v1 cases to the four exact production probes", () => {
    const exactTriggers = Object.keys(PHRASE_V1_COMPLETIONS).sort();
    expect(exactTriggers).toEqual([
      "looking forward",
      "please",
      "thank you",
      "the next step",
    ]);
    for (const entry of SCENARIO_PLAN.filter(
      (candidate) => candidate.provider === "phrase_v1",
    )) {
      for (const testCase of entry.cases) {
        expect(exactTriggers).toContain(testCase.trigger);
        expect(testCase.expected_output).toBe(
          PHRASE_V1_COMPLETIONS[testCase.trigger],
        );
      }
    }
  });

  it("rejects incompatible or incomplete scenario declarations", () => {
    const valid = scenarioDefinition("chromium.full-chain");
    expect(() =>
      validateScenarioPlan([
        {
          ...valid,
          cases: [{ trigger: "arbitrary prose", expected_output: " fallback" }],
        },
      ]),
    ).toThrow(/unsupported phrase_v1 trigger/u);
    expect(() =>
      validateScenarioPlan([
        {
          ...valid,
          cases: [{ trigger: "thank you", expected_output: " wrong" }],
        },
      ]),
    ).toThrow(/expected output mismatch/u);
    expect(() =>
      validateScenarioPlan([{ ...valid, provider: "unlabeled-provider" }]),
    ).toThrow(/unknown provider/u);
    expect(() =>
      validateScenarioPlan([
        { ...valid, evidence_class: "live-browser-fault-host" },
      ]),
    ).toThrow(/provider\/evidence class mismatch/u);
    expect(() =>
      validateScenarioPlan([
        { ...valid, target: "https://unapproved.invalid/" },
      ]),
    ).toThrow(/unapproved target/u);
    expect(() =>
      validateScenarioPlan([
        {
          id: "missing-target",
          provider: "phrase_v1",
          evidence_class: "real-rust-chain",
          cases: [{ trigger: "thank you", expected_output: " for your time" }],
        },
      ]),
    ).toThrow(/target must be a nonempty string/u);
  });

  it("requires runtime results to cover the plan exactly once", () => {
    const ids = SCENARIO_PLAN.map((entry) => entry.id);
    expect(() => validateCompletedScenarioIds(ids)).not.toThrow();
    expect(() => validateCompletedScenarioIds(ids.slice(1))).toThrow(
      /do not exactly match/u,
    );
    expect(() => validateCompletedScenarioIds([...ids, ids[0]!])).toThrow(
      /do not exactly match/u,
    );
  });
});
