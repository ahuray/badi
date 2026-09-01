export interface ProductLiveStatus {
  readonly metrics: Readonly<Record<string, number>>;
}

export interface ProductLiveBrowserLifecycle {
  beginCleanup(): void;
  fail(stage: string, message: string, action: string): void;
  throwIfFailed(): void;
  race<T>(operation: Promise<T>): Promise<T>;
}

export class ProductLiveStageError extends Error {
  readonly stage: string;
  readonly action: string;
}

export function createBrowserLifecycle(): ProductLiveBrowserLifecycle;
export function interactiveEvidenceDeltas(
  before: ProductLiveStatus,
  after: ProductLiveStatus,
): Readonly<Record<string, number>>;
export function requireInteractiveEvidence(
  before: ProductLiveStatus,
  after: ProductLiveStatus,
): Readonly<Record<string, number>>;
