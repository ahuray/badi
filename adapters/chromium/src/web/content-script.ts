import { startBrowserContent } from '../content/browser-content';
import { webOrigin } from './boundary';

const scope = globalThis as typeof globalThis & { badiWebRefresh?: () => void };
if (scope.badiWebRefresh) {
  scope.badiWebRefresh();
} else {
  const refresh = startBrowserContent(document => document.defaultView === document.defaultView?.top &&
    webOrigin(document.location.href) === document.location.origin, 5000);
  if (refresh) scope.badiWebRefresh = refresh;
}
