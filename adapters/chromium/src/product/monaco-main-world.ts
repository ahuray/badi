export interface MonacoGeometry {
  readonly left: number;
  readonly top: number;
  readonly height: number;
}

export interface MonacoSnapshotGuard {
  readonly modelUri: string;
  readonly languageId: "markdown";
  readonly versionId: number;
  readonly valueLength: number;
  readonly offset: number;
  readonly lineNumber: number;
  readonly column: number;
  readonly before: string;
  readonly after: string;
}

export interface MonacoSnapshot extends MonacoSnapshotGuard {
  readonly geometry: MonacoGeometry;
}

/**
 * Serialized by chrome.scripting.executeScript. Keep this function closure-free:
 * no imported helpers or module constants may be referenced at runtime.
 */
export function readDillingerMonacoSnapshotInMainWorld(): MonacoSnapshot | null {
  if (
    globalThis.location.href !== "https://dillinger.io/" ||
    globalThis.document.visibilityState !== "visible" ||
    !globalThis.document.hasFocus()
  ) {
    return null;
  }
  const candidate = globalThis as typeof globalThis & {
    monaco?: {
      editor?: {
        getEditors?: () => unknown[];
        getModels?: () => unknown[];
      };
    };
  };
  const editors = candidate.monaco?.editor?.getEditors?.();
  const models = candidate.monaco?.editor?.getModels?.();
  if (!Array.isArray(editors) || editors.length !== 1 || !Array.isArray(models) || models.length !== 1) {
    return null;
  }
  const editor = editors[0] as {
    getModel?: () => unknown;
    getPosition?: () => { lineNumber: number; column: number } | null;
    getSelection?: () => {
      startLineNumber: number;
      startColumn: number;
      endLineNumber: number;
      endColumn: number;
    } | null;
    getScrolledVisiblePosition?: (
      position: { lineNumber: number; column: number },
    ) => { left: number; top: number; height: number } | null;
    getDomNode?: () => HTMLElement | null;
    hasTextFocus?: () => boolean;
  };
  const model = models[0] as {
    uri?: { toString(): string };
    getLanguageId?: () => string;
    getVersionId?: () => number;
    getValue?: () => string;
    getOffsetAt?: (position: { lineNumber: number; column: number }) => number;
  };
  if (
    editor.getModel?.() !== model ||
    editor.hasTextFocus?.() !== true ||
    model.getLanguageId?.() !== "markdown"
  ) {
    return null;
  }
  const position = editor.getPosition?.();
  const selection = editor.getSelection?.();
  const domNode = editor.getDomNode?.();
  if (
    position === null ||
    position === undefined ||
    selection === null ||
    selection === undefined ||
    domNode === null ||
    domNode === undefined ||
    selection.startLineNumber !== selection.endLineNumber ||
    selection.startColumn !== selection.endColumn ||
    selection.startLineNumber !== position.lineNumber ||
    selection.startColumn !== position.column
  ) {
    return null;
  }
  const value = model.getValue?.();
  const versionId = model.getVersionId?.();
  const offset = model.getOffsetAt?.(position);
  const modelUri = model.uri?.toString();
  const visible = editor.getScrolledVisiblePosition?.(position);
  const rect = domNode.getBoundingClientRect();
  if (
    typeof value !== "string" ||
    !Number.isSafeInteger(versionId) ||
    (versionId ?? -1) < 0 ||
    !Number.isSafeInteger(offset) ||
    (offset ?? -1) < 0 ||
    (offset ?? value.length + 1) > value.length ||
    typeof modelUri !== "string" ||
    modelUri.length === 0 ||
    visible === null ||
    visible === undefined
  ) {
    return null;
  }
  const left = rect.left + visible.left;
  const top = rect.top + visible.top;
  const height = visible.height;
  if (
    ![left, top, height].every(Number.isFinite) ||
    height <= 0 ||
    left < Math.max(0, rect.left) ||
    top < Math.max(0, rect.top) ||
    left >= Math.min(globalThis.innerWidth, rect.right) ||
    top + height > Math.min(globalThis.innerHeight, rect.bottom)
  ) {
    return null;
  }
  return {
    modelUri,
    languageId: "markdown",
    versionId: versionId as number,
    valueLength: value.length,
    offset: offset as number,
    lineNumber: position.lineNumber,
    column: position.column,
    before: Array.from(value.slice(0, offset as number)).slice(-512).join(""),
    after: Array.from(value.slice(offset as number)).slice(0, 128).join(""),
    geometry: { left, top, height },
  };
}

/**
 * Performs one target-native edit and brackets it with Monaco undo stops. The
 * exact document, model, version, caret, and bounded text guard are all checked
 * immediately before mutation. Keep this function closure-free.
 */
export function applyDillingerMonacoEditInMainWorld(
  expected: MonacoSnapshotGuard,
  text: string,
): boolean {
  if (
    globalThis.location.href !== "https://dillinger.io/" ||
    globalThis.document.visibilityState !== "visible" ||
    !globalThis.document.hasFocus() ||
    text.length === 0
  ) {
    return false;
  }
  const candidate = globalThis as typeof globalThis & {
    monaco?: {
      Range?: new (
        startLineNumber: number,
        startColumn: number,
        endLineNumber: number,
        endColumn: number,
      ) => unknown;
      editor?: {
        getEditors?: () => unknown[];
        getModels?: () => unknown[];
      };
    };
  };
  const editors = candidate.monaco?.editor?.getEditors?.();
  const models = candidate.monaco?.editor?.getModels?.();
  const Range = candidate.monaco?.Range;
  if (
    !Array.isArray(editors) ||
    editors.length !== 1 ||
    !Array.isArray(models) ||
    models.length !== 1 ||
    typeof Range !== "function"
  ) {
    return false;
  }
  const editor = editors[0] as {
    getModel?: () => unknown;
    getPosition?: () => { lineNumber: number; column: number } | null;
    getSelection?: () => {
      startLineNumber: number;
      startColumn: number;
      endLineNumber: number;
      endColumn: number;
    } | null;
    hasTextFocus?: () => boolean;
    pushUndoStop?: () => boolean;
    executeEdits?: (
      source: string,
      edits: Array<{ range: unknown; text: string; forceMoveMarkers: boolean }>,
    ) => boolean;
    trigger?: (source: string, handlerId: string, payload: unknown) => void;
  };
  const model = models[0] as {
    uri?: { toString(): string };
    getLanguageId?: () => string;
    getVersionId?: () => number;
    getValue?: () => string;
    getOffsetAt?: (position: { lineNumber: number; column: number }) => number;
  };
  const position = editor.getPosition?.();
  const selection = editor.getSelection?.();
  const value = model.getValue?.();
  const offset = position === null || position === undefined ? undefined : model.getOffsetAt?.(position);
  if (
    editor.getModel?.() !== model ||
    editor.hasTextFocus?.() !== true ||
    model.getLanguageId?.() !== "markdown" ||
    model.uri?.toString() !== expected.modelUri ||
    model.getVersionId?.() !== expected.versionId ||
    typeof value !== "string" ||
    value.length !== expected.valueLength ||
    position === null ||
    position === undefined ||
    selection === null ||
    selection === undefined ||
    position.lineNumber !== expected.lineNumber ||
    position.column !== expected.column ||
    selection.startLineNumber !== selection.endLineNumber ||
    selection.startColumn !== selection.endColumn ||
    selection.startLineNumber !== position.lineNumber ||
    selection.startColumn !== position.column ||
    offset !== expected.offset ||
    Array.from(value.slice(0, expected.offset)).slice(-512).join("") !== expected.before ||
    Array.from(value.slice(expected.offset)).slice(0, 128).join("") !== expected.after
  ) {
    return false;
  }

  const originalValue = value;
  const expectedValue =
    originalValue.slice(0, expected.offset) + text + originalValue.slice(expected.offset);
  const range = new Range(
    expected.lineNumber,
    expected.column,
    expected.lineNumber,
    expected.column,
  );
  editor.pushUndoStop?.();
  const executed = editor.executeEdits?.("badi.dillinger.accept", [
    { range, text, forceMoveMarkers: true },
  ]);
  editor.pushUndoStop?.();
  if (executed !== true || model.getValue?.() !== expectedValue) {
    if (model.getValue?.() !== originalValue) {
      editor.trigger?.("badi.dillinger.accept", "undo", null);
    }
    return false;
  }
  return true;
}
