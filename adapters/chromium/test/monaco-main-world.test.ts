// @vitest-environment-options {"url":"https://dillinger.io/"}

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applyDillingerMonacoEditInMainWorld,
  readDillingerMonacoSnapshotInMainWorld,
  type MonacoSnapshot,
} from "../src/product/monaco-main-world";

interface MonacoHarness {
  readonly editor: {
    executeEdits: ReturnType<typeof vi.fn>;
    getDomNode(): HTMLElement;
    getModel(): object;
    getPosition(): { lineNumber: number; column: number };
    getScrolledVisiblePosition(): { left: number; top: number; height: number };
    getSelection(): {
      startLineNumber: number;
      startColumn: number;
      endLineNumber: number;
      endColumn: number;
    };
    hasTextFocus(): boolean;
    pushUndoStop: ReturnType<typeof vi.fn>;
    trigger: ReturnType<typeof vi.fn>;
  };
  readonly getValue: () => string;
  readonly getVersion: () => number;
  readonly setDocumentFocus: (focused: boolean) => void;
  readonly setDocumentVisibility: (visibility: DocumentVisibilityState) => void;
  readonly setVersion: (version: number) => void;
}

function installMonaco(
  initialValue = "thank you",
  behavior: "normal" | "corrupt" = "normal",
): MonacoHarness {
  let value = initialValue;
  let version = 7;
  let offset = value.length;
  let undoValue: string | null = null;
  let documentFocused = true;
  let documentVisibility: DocumentVisibilityState = "visible";
  vi.spyOn(document, "hasFocus").mockImplementation(() => documentFocused);
  vi.spyOn(document, "visibilityState", "get").mockImplementation(
    () => documentVisibility,
  );
  class Range {
    constructor(
      readonly startLineNumber: number,
      readonly startColumn: number,
      readonly endLineNumber: number,
      readonly endColumn: number,
    ) {}
  }
  const domNode = document.createElement("div");
  domNode.getBoundingClientRect = () =>
    ({ left: 10, top: 20, right: 700, bottom: 500, width: 690, height: 480 }) as DOMRect;
  const model = {
    uri: { toString: () => "inmemory://model/1" },
    getLanguageId: () => "markdown",
    getVersionId: () => version,
    getValue: () => value,
    getOffsetAt: () => offset,
  };
  const editor = {
    getModel: () => model,
    hasTextFocus: () => true,
    getPosition: () => ({ lineNumber: 1, column: offset + 1 }),
    getSelection: () => ({
      startLineNumber: 1,
      startColumn: offset + 1,
      endLineNumber: 1,
      endColumn: offset + 1,
    }),
    getScrolledVisiblePosition: () => ({ left: 100, top: 40, height: 18 }),
    getDomNode: () => domNode,
    pushUndoStop: vi.fn(() => true),
    executeEdits: vi.fn(
      (
        _source: string,
        edits: Array<{ range: Range; text: string }>,
      ): boolean => {
        const edit = edits[0];
        if (edit === undefined) return false;
        undoValue = value;
        const insertionOffset = edit.range.startColumn - 1;
        value =
          behavior === "corrupt"
            ? `${value}!corrupt`
            : value.slice(0, insertionOffset) + edit.text + value.slice(insertionOffset);
        offset = behavior === "corrupt" ? value.length : insertionOffset + edit.text.length;
        version += 1;
        return true;
      },
    ),
    trigger: vi.fn((_source: string, handler: string) => {
      if (handler === "undo" && undoValue !== null) {
        value = undoValue;
        offset = value.length;
        version += 1;
        undoValue = null;
      }
    }),
  };
  Object.defineProperty(globalThis, "monaco", {
    configurable: true,
    value: {
      Range,
      editor: {
        getEditors: () => [editor],
        getModels: () => [model],
      },
    },
  });
  return {
    editor,
    getValue: () => value,
    getVersion: () => version,
    setDocumentFocus: (focused) => {
      documentFocused = focused;
    },
    setDocumentVisibility: (visibility) => {
      documentVisibility = visibility;
    },
    setVersion: (next) => {
      version = next;
    },
  };
}

afterEach(() => {
  delete (globalThis as { monaco?: unknown }).monaco;
  history.replaceState(null, "", "/");
  vi.restoreAllMocks();
});

describe("Dillinger MAIN-world Monaco adapter", () => {
  it("captures one focused Markdown model and visible caret geometry", () => {
    installMonaco("hello 😀 thank you");
    const snapshot = readDillingerMonacoSnapshotInMainWorld();
    expect(snapshot).toEqual({
      modelUri: "inmemory://model/1",
      languageId: "markdown",
      versionId: 7,
      valueLength: "hello 😀 thank you".length,
      offset: "hello 😀 thank you".length,
      lineNumber: 1,
      column: "hello 😀 thank you".length + 1,
      before: "hello 😀 thank you",
      after: "",
      geometry: { left: 110, top: 60, height: 18 },
    });
  });

  it("uses one Monaco transaction that target-native undo restores exactly", () => {
    const harness = installMonaco();
    const snapshot = readDillingerMonacoSnapshotInMainWorld() as MonacoSnapshot;
    expect(applyDillingerMonacoEditInMainWorld(snapshot, " for your time")).toBe(true);
    expect(harness.getValue()).toBe("thank you for your time");
    expect(harness.editor.pushUndoStop).toHaveBeenCalledTimes(2);
    expect(harness.editor.executeEdits).toHaveBeenCalledWith(
      "badi.dillinger.accept",
      [
        {
          range: expect.objectContaining({
            startLineNumber: 1,
            startColumn: 10,
            endLineNumber: 1,
            endColumn: 10,
          }),
          text: " for your time",
          forceMoveMarkers: true,
        },
      ],
    );

    harness.editor.trigger("test", "undo", null);
    expect(harness.getValue()).toBe("thank you");
  });

  it("refuses a stale version or stale exact-document URL before mutation", () => {
    const harness = installMonaco();
    const snapshot = readDillingerMonacoSnapshotInMainWorld() as MonacoSnapshot;
    harness.setVersion(harness.getVersion() + 1);
    expect(applyDillingerMonacoEditInMainWorld(snapshot, " stale")).toBe(false);
    expect(harness.editor.executeEdits).not.toHaveBeenCalled();

    harness.setVersion(snapshot.versionId);
    history.replaceState(null, "", "/?navigated=1");
    expect(applyDillingerMonacoEditInMainWorld(snapshot, " stale")).toBe(false);
    expect(harness.editor.executeEdits).not.toHaveBeenCalled();
    expect(harness.getValue()).toBe("thank you");
  });

  it("refuses context reads and mutation after the document becomes hidden", () => {
    const harness = installMonaco();
    const snapshot = readDillingerMonacoSnapshotInMainWorld() as MonacoSnapshot;

    harness.setDocumentVisibility("hidden");

    expect(readDillingerMonacoSnapshotInMainWorld()).toBeNull();
    expect(applyDillingerMonacoEditInMainWorld(snapshot, " blocked")).toBe(false);
    expect(harness.editor.executeEdits).not.toHaveBeenCalled();
    expect(harness.getValue()).toBe("thank you");
  });

  it("refuses context reads and mutation after the document loses focus", () => {
    const harness = installMonaco();
    const snapshot = readDillingerMonacoSnapshotInMainWorld() as MonacoSnapshot;

    harness.setDocumentFocus(false);

    expect(readDillingerMonacoSnapshotInMainWorld()).toBeNull();
    expect(applyDillingerMonacoEditInMainWorld(snapshot, " blocked")).toBe(false);
    expect(harness.editor.executeEdits).not.toHaveBeenCalled();
    expect(harness.getValue()).toBe("thank you");
  });

  it("undoes its own edit when post-mutation verification fails", () => {
    const harness = installMonaco("thank you", "corrupt");
    const snapshot = readDillingerMonacoSnapshotInMainWorld() as MonacoSnapshot;
    expect(applyDillingerMonacoEditInMainWorld(snapshot, " expected")).toBe(false);
    expect(harness.editor.trigger).toHaveBeenCalledWith(
      "badi.dillinger.accept",
      "undo",
      null,
    );
    expect(harness.getValue()).toBe("thank you");
  });
});
