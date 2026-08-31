import { DILLINGER_PERMISSION_PATTERN } from "./target";

export interface ProductPermissionsApi {
  contains(permissions: chrome.permissions.Permissions): Promise<boolean>;
  request(permissions: chrome.permissions.Permissions): Promise<boolean>;
  remove(permissions: chrome.permissions.Permissions): Promise<boolean>;
}

export const DILLINGER_PERMISSION = Object.freeze({
  origins: Object.freeze([DILLINGER_PERMISSION_PATTERN]),
});

function permissionRequest(): chrome.permissions.Permissions {
  return { origins: [...DILLINGER_PERMISSION.origins] };
}

export function hasDillingerAccess(
  permissions: ProductPermissionsApi = chrome.permissions,
): Promise<boolean> {
  return permissions.contains(permissionRequest());
}

export function requestDillingerAccess(
  permissions: ProductPermissionsApi = chrome.permissions,
): Promise<boolean> {
  // Callers must invoke this synchronously from an extension-page user gesture.
  return permissions.request(permissionRequest());
}

export function removeDillingerAccess(
  permissions: ProductPermissionsApi = chrome.permissions,
): Promise<boolean> {
  return permissions.remove(permissionRequest());
}

export function permissionSetIncludesDillinger(
  permissions: chrome.permissions.Permissions,
): boolean {
  return permissions.origins?.includes(DILLINGER_PERMISSION_PATTERN) === true;
}
