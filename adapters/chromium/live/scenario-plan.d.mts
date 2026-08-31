export const PHRASE_V1_PROVIDER: "phrase_v1";
export const FIXTURE_FAULT_PROVIDER: "fixture_fault_v1";
export const PHRASE_V1_COMPLETIONS: Readonly<Record<string, string>>;

export interface ProviderCase {
  readonly trigger: string;
  readonly expected_output: string;
}

export interface ScenarioDefinition {
  readonly id: string;
  readonly provider: string;
  readonly evidence_class: string;
  readonly target: string;
  readonly cases: readonly ProviderCase[];
}

export const SCENARIO_PLAN: readonly ScenarioDefinition[];

export function validateScenarioPlan(
  plan?: readonly unknown[],
): readonly ScenarioDefinition[];
export function scenarioDefinition(id: string): ScenarioDefinition;
export function scenarioCase(id: string, index?: number): ProviderCase;
export function validateCompletedScenarioIds(ids: Iterable<string>): void;
