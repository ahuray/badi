import { describe, expect, it, vi } from "vitest";
import {
  DILLINGER_PERMISSION,
  hasDillingerAccess,
  permissionSetIncludesDillinger,
  removeDillingerAccess,
  requestDillingerAccess,
  type ProductPermissionsApi,
} from "../src/product/permissions";
import { DILLINGER_PERMISSION_PATTERN } from "../src/product/target";

function permissionsApi(result: boolean): ProductPermissionsApi & {
  contains: ReturnType<typeof vi.fn>;
  request: ReturnType<typeof vi.fn>;
  remove: ReturnType<typeof vi.fn>;
} {
  return {
    contains: vi.fn().mockResolvedValue(result),
    request: vi.fn().mockResolvedValue(result),
    remove: vi.fn().mockResolvedValue(result),
  };
}

describe("Dillinger optional permission lifecycle", () => {
  it("requests only the exact optional origin and preserves grant or denial", async () => {
    const granted = permissionsApi(true);
    const denied = permissionsApi(false);

    await expect(requestDillingerAccess(granted)).resolves.toBe(true);
    await expect(requestDillingerAccess(denied)).resolves.toBe(false);
    expect(granted.request).toHaveBeenCalledWith({
      origins: [DILLINGER_PERMISSION_PATTERN],
    });
    expect(denied.request).toHaveBeenCalledWith({
      origins: [DILLINGER_PERMISSION_PATTERN],
    });
    expect(DILLINGER_PERMISSION).toEqual({ origins: [DILLINGER_PERMISSION_PATTERN] });
  });

  it("checks and revokes only the exact optional origin", async () => {
    const api = permissionsApi(true);
    await expect(hasDillingerAccess(api)).resolves.toBe(true);
    await expect(removeDillingerAccess(api)).resolves.toBe(true);
    expect(api.contains).toHaveBeenCalledWith({
      origins: [DILLINGER_PERMISSION_PATTERN],
    });
    expect(api.remove).toHaveBeenCalledWith({
      origins: [DILLINGER_PERMISSION_PATTERN],
    });
    expect(
      permissionSetIncludesDillinger({ origins: [DILLINGER_PERMISSION_PATTERN] }),
    ).toBe(true);
    expect(permissionSetIncludesDillinger({ origins: ["https://*/*"] })).toBe(false);
    expect(permissionSetIncludesDillinger({})).toBe(false);
  });
});
