.pragma library
// Pure settings validation, status ladder and policy mutations for the Omarchy
// panel, free of QML so Node tests exercise every branch without a display
// (tests/settings-model.test.mjs).

var WRITING_ORDER = ["en", "de", "fa", "fr", "es", "pt"]
// The panel offers European writing languages only; Persian stays terminal-only
// and survives panel writes untouched.
var PANEL_WRITING_ORDER = ["en", "de", "fr", "es", "pt"]
var SPELLING_ORDER = ["en", "de", "fr", "es", "it", "nl", "pl"]
var DEFAULT_RETENTION_DAYS = 30
var MAX_SUBJECTS = 64

// Browsers share website rules (`browser_origin`), while `language_overrides`
// keys by each browser's canonical `app_id`.
var BROWSERS = [
  {name: "Chromium", appId: "chromium", desktop: ["chromium", "org.chromium.Chromium"]},
  {name: "Brave", appId: "brave-origin", desktop: ["brave-origin", "brave-browser", "com.brave.Browser"]},
  {name: "Zen", appId: "zen", desktop: ["zen", "app.zen_browser.zen"]},
  {name: "Zen Twilight", appId: "zen-twilight", desktop: ["zen-twilight", "app.zen_browser.twilight"], optional: true}
]

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function objectValue(parent, key) {
  return isObject(parent) && isObject(parent[key]) ? parent[key] : ({})
}

function hasBoolean(parent, key) {
  return isObject(parent) && typeof parent[key] === "boolean"
}

function boolValue(parent, key, fallback) {
  return hasBoolean(parent, key) ? parent[key] : fallback
}

function integerValue(parent, key, fallback) {
  return isObject(parent) && Number.isSafeInteger(parent[key]) ? parent[key] : fallback
}

function hasExactKeys(value, keys) {
  if (!isObject(value)) return false
  var actual = Object.keys(value).sort()
  var expected = keys.slice().sort()
  if (actual.length !== expected.length) return false
  for (var index = 0; index < expected.length; index += 1) {
    if (actual[index] !== expected[index]) return false
  }
  return true
}

function isLinuxAppId(value) {
  return typeof value === "string"
    && value.length <= 128
    && /^[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)*$/.test(value)
}

function isDecision(value) {
  return value === "allow" || value === "block"
}

function isRetention(value) {
  if (!isObject(value) || typeof value.mode !== "string") return false
  if (value.mode === "none") return hasExactKeys(value, ["mode"])
  return value.mode === "bounded"
    && hasExactKeys(value, ["mode", "days"])
    && Number.isSafeInteger(value.days)
    && value.days >= 1
    && value.days <= 90
}

function isBrowserIdentity(value) {
  return hasExactKeys(value, ["kind", "adapter", "scheme", "host", "port"])
    && value.kind === "browser_origin"
    && value.adapter === "chromium"
    && (value.scheme === "http" || value.scheme === "https")
    && typeof value.host === "string"
    && value.host.length > 0
    && value.host.length <= 253
    && /^[a-z0-9.:-]+$/.test(value.host)
    && Number.isSafeInteger(value.port)
    && value.port >= 1
    && value.port <= 65535
}

function isLinuxIdentity(value) {
  return hasExactKeys(value, ["kind", "adapter", "app_id"])
    && value.kind === "linux_app"
    && (value.adapter === "fcitx" || value.adapter === "obsidian" || value.adapter === "shell"
      || value.adapter === "neovim" || value.adapter === "libreoffice")
    && isLinuxAppId(value.app_id)
}

function isIdentity(value) {
  return isBrowserIdentity(value) || isLinuxIdentity(value)
}

function isPermissions(value) {
  if (!hasExactKeys(value,
      ["suggest", "display", "context_read", "learn", "retention"])) return false
  if (!isDecision(value.suggest)
      || !isDecision(value.display)
      || !isDecision(value.context_read)
      || !isDecision(value.learn)
      || !isRetention(value.retention)) return false
  if (value.suggest === "allow"
      && (value.display !== "allow" || value.context_read !== "allow")) return false
  if (value.learn === "allow"
      && (value.suggest !== "allow"
        || value.display !== "allow"
        || value.context_read !== "allow")) return false
  return value.retention.mode !== "bounded" || value.learn === "allow"
}

function isTerminalIdentity(value) {
  return isObject(value)
    && value.kind === "linux_app"
    && (value.adapter === "shell"
      || (value.adapter === "fcitx"
        && (value.app_id === "com.mitchellh.ghostty"
          || value.app_id === "foot"
          || value.app_id === "org.codeberg.dnkl.foot")))
}

function isSubject(value) {
  return hasExactKeys(value, ["identity", "permissions"])
    && isIdentity(value.identity)
    && isPermissions(value.permissions)
    && (!isTerminalIdentity(value.identity) || value.permissions.learn === "block")
}

function compareIdentities(left, right) {
  var kindOrder = {"browser_origin": 0, "linux_app": 1}
  var leftKind = kindOrder[left.kind]
  var rightKind = kindOrder[right.kind]
  if (leftKind !== rightKind) return leftKind - rightKind
  var fields = left.kind === "browser_origin"
    ? ["adapter", "scheme", "host"]
    : ["adapter", "app_id"]
  for (var index = 0; index < fields.length; index += 1) {
    var field = fields[index]
    if (left[field] < right[field]) return -1
    if (left[field] > right[field]) return 1
  }
  return left.kind === "browser_origin" ? left.port - right.port : 0
}

// Codes in `order` without repeats, in that order.
function isOrderedCodes(value, order) {
  if (!Array.isArray(value)) return false
  for (var index = 0; index < value.length; index += 1) {
    var position = order.indexOf(value[index])
    if (position < 0 || (index > 0 && position <= order.indexOf(value[index - 1]))) return false
  }
  return true
}

// Whether `value` is one the settings schema
// (broker/schemas/badi.settings.v2.schema.json) allows for the optional key
// `key`. Producers omit a key at its default. An unrecognized key fails the
// whole document, so the panel never writes over a setting it cannot see;
// tests keep this list equal to the schema's.
function isOptionalSetting(key, value) {
  switch (key) {
  case "all_web_origins":
  case "all_linux_apps":
  case "mid_line":
  case "style_learning":
  case "context_words":
  case "languages_chosen":
  case "places_chosen":
    return typeof value === "boolean"
  case "escape_quiet_seconds":
    return Number.isSafeInteger(value) && value >= 1 && value <= 3600
  case "battery_mode":
    return ["normal", "quiet", "explicit"].indexOf(value) >= 0
  case "screen_share":
    return value === "hold" || value === "suggest"
  case "suggestion_length":
    return value === "short" || value === "normal"
  case "spelling_languages":
    return isOrderedCodes(value, SPELLING_ORDER)
  case "writing_languages":
    return isOrderedCodes(value, WRITING_ORDER)
  case "trailing_space_healing":
    return isObject(value) && Object.keys(value).every(function(code) {
      return WRITING_ORDER.indexOf(code) >= 0 && typeof value[code] === "boolean"
    })
  case "writing_model":
    return typeof value === "string" && /^[a-z0-9.-]{1,64}$/.test(value)
  case "context_words_off":
    // In the broker's identity order.
    if (!Array.isArray(value) || value.length === 0 || value.length > MAX_SUBJECTS) return false
    for (var index = 0; index < value.length; index += 1) {
      if (!isIdentity(value[index])
          || (index > 0 && compareIdentities(value[index - 1], value[index]) >= 0)) return false
    }
    return true
  case "language_overrides":
    return isObject(value) && Object.keys(value).length <= MAX_SUBJECTS && Object.keys(value).every(function(app) {
      return isLinuxAppId(app) && WRITING_ORDER.indexOf(value[app]) >= 0
    })
  default:
    return false
  }
}

function isSettingsDocument(value) {
  if (!isObject(value)) return false
  var required = ["schema", "revision", "paused", "subjects"]
  var keys = Object.keys(value)
  for (var key = 0; key < keys.length; key += 1) {
    if (required.indexOf(keys[key]) < 0 && !isOptionalSetting(keys[key], value[keys[key]])) return false
  }
  if (value.schema !== "badi.settings.v2"
      || !Number.isSafeInteger(value.revision)
      || value.revision < 0
      || typeof value.paused !== "boolean"
      || !Array.isArray(value.subjects)
      || value.subjects.length > MAX_SUBJECTS) return false
  for (var index = 0; index < value.subjects.length; index += 1) {
    if (!isSubject(value.subjects[index])) return false
    if (index > 0 && compareIdentities(
        value.subjects[index - 1].identity,
        value.subjects[index].identity) >= 0) return false
  }
  return true
}

// One status ladder for the bar mark, IPC state, activity fallback and hero
// pill: loading -> offline -> degraded -> screen-share hold -> paused ->
// no app allowed -> ready.
function status(state) {
  var s = isObject(state) ? state : ({})
  var reachable = s.brokerReachable === true
  var brokerPaused = s.brokerPaused === true
  var degraded = s.controlPlaneDegraded === true
  var settingsPaused = s.settingsPaused === true
  var sharingHeld = s.sharingHeld === true
  var anyApp = s.anyAppEnabled === true
  var active = typeof s.serviceActive === "string" ? s.serviceActive : ""
  if (!reachable && (active === "active" || active === "activating")) {
    return {kind: "loading", ready: false, paused: false, loading: true,
      label: "Model starting or unavailable",
      mode: {glyph: "◌", text: "Loading", tone: "dim"}}
  }
  if (!reachable) {
    return {kind: "offline", ready: false, paused: false, loading: false,
      label: "Model offline",
      mode: {glyph: "○", text: "Model offline", tone: "dim"}}
  }
  if (degraded) {
    return {kind: "degraded", ready: false, paused: brokerPaused, loading: false,
      label: "Settings need attention",
      mode: {glyph: "!", text: "Settings need attention", tone: "urgent"}}
  }
  if (brokerPaused && sharingHeld && !settingsPaused) {
    return {kind: "held", ready: false, paused: true, loading: false,
      label: "Held while the screen is shared",
      mode: {glyph: "⏸", text: "Held · screen shared", tone: "dim"}}
  }
  if (brokerPaused) {
    return {kind: "paused", ready: false, paused: true, loading: false,
      label: "Predictions paused",
      mode: {glyph: "⏸", text: "Paused", tone: "dim"}}
  }
  if (!anyApp) {
    return {kind: "no_apps", ready: false, paused: false, loading: false,
      label: "Applications disabled",
      mode: {glyph: "◇", text: "No app allowed", tone: "dim"}}
  }
  return {kind: "ready", ready: true, paused: false, loading: false,
    label: "Model ready",
    mode: {glyph: "⏵⏵", text: "Suggesting", tone: "accent"}}
}

function appAdapter(appId) {
  return appId === "obsidian" ? "obsidian" : appId === "bash" ? "shell" : appId === "nvim" ? "neovim"
    : appId === "libreoffice-writer" ? "libreoffice" : "fcitx"
}

function appIdentity(appId) {
  return {kind: "linux_app", adapter: appAdapter(appId), app_id: appId}
}

function originText(identity) {
  var standard = identity.scheme === "https" ? 443 : 80
  return identity.scheme + "://" + identity.host + (identity.port === standard ? "" : ":" + identity.port)
}

// An exact http(s) origin, as `badi site` accepts it (ASCII hosts only here).
function siteIdentity(text) {
  var match = /^(https?):\/\/([a-z0-9.-]+)(?::([0-9]{1,5}))?\/?$/.exec(String(text || "").trim().toLowerCase())
  if (!match) return null
  var port = match[3] ? Number(match[3]) : (match[1] === "https" ? 443 : 80)
  if (port < 1 || port > 65535 || match[2].length > 253) return null
  return {kind: "browser_origin", adapter: "chromium", scheme: match[1], host: match[2], port: port}
}

function identityLabel(identity, supportedApps) {
  if (!isObject(identity)) return ""
  if (identity.kind === "linux_app") {
    var apps = Array.isArray(supportedApps) ? supportedApps : []
    var app = apps.find(function(item) { return item.appId === identity.app_id })
    if (app) return app.name
    var browser = BROWSERS.find(function(item) { return item.appId === identity.app_id })
    return browser ? browser.name : identity.app_id
  }
  return originText(identity)
}

function allows(permissions) {
  return isObject(permissions)
    && permissions.context_read === "allow"
    && permissions.suggest === "allow"
    && permissions.display === "allow"
}

function learns(permissions) {
  return allows(permissions)
    && permissions.learn === "allow"
    && isRetention(permissions.retention)
    && permissions.retention.mode === "bounded"
}

function learningDays(permissions) {
  return learns(permissions) ? permissions.retention.days : DEFAULT_RETENTION_DAYS
}

function subjectRule(document, identity) {
  var subjects = isObject(document) && Array.isArray(document.subjects) ? document.subjects : []
  for (var index = 0; index < subjects.length; index += 1) {
    var entry = subjects[index]
    if (isObject(entry) && isIdentity(entry.identity)
        && compareIdentities(entry.identity, identity) === 0) return entry
  }
  return null
}

function appRule(document, appId) {
  return subjectRule(document, appIdentity(appId))
}

// Effective app permission: an exact rule, else the app list mode (never for manual apps).
function appEnabled(document, supportedApps, appId) {
  var rule = appRule(document, appId)
  if (rule) return allows(rule.permissions)
  var apps = Array.isArray(supportedApps) ? supportedApps : []
  var app = apps.find(function(item) { return item.appId === appId })
  return isObject(document) && document.all_linux_apps === true && !(app && app.manual)
}

function siteRules(document) {
  var subjects = isObject(document) && Array.isArray(document.subjects) ? document.subjects : []
  var rules = []
  for (var index = 0; index < subjects.length; index += 1) {
    var entry = subjects[index]
    if (!isObject(entry) || !isBrowserIdentity(entry.identity)) continue
    rules.push({
      identity: entry.identity,
      allowed: allows(entry.permissions),
      learning: learns(entry.permissions),
      days: learningDays(entry.permissions),
      origin: originText(entry.identity)
    })
  }
  return rules
}

function anySiteEnabled(document) {
  return (isObject(document) && document.all_web_origins === true)
    || siteRules(document).some(function(rule) { return rule.allowed })
}

function anyAppEnabled(document, supportedApps) {
  var apps = Array.isArray(supportedApps) ? supportedApps : []
  var subjects = isObject(document) && Array.isArray(document.subjects) ? document.subjects : []
  return (isObject(document) && document.all_linux_apps === true)
    || anySiteEnabled(document)
    || apps.some(function(app) { return appEnabled(document, apps, app.appId) })
    || subjects.some(function(entry) {
      return isObject(entry) && isLinuxIdentity(entry.identity) && allows(entry.permissions)
    })
}

// `list` with `code` turned on or off, in the broker's `order`.
function withLanguage(list, order, code, on) {
  return order.filter(function(item) { return item === code ? on : list.indexOf(item) >= 0 })
}

// Each language list, omitted at its default as the broker writes it.
function putSpelling(document, languages) {
  if (languages.join() === "en") delete document.spelling_languages
  else document.spelling_languages = languages
}

function putWriting(document, languages) {
  if (languages.join() === "en,de") delete document.writing_languages
  else document.writing_languages = languages
}

// `document` with the exact rule for `identity` replaced, or removed for a null
// decision, in the broker's identity order. Blocking revokes learning; keeping
// an allowed non-terminal rule preserves its learning grant and retention.
function putRule(document, identity, decision) {
  var existing = subjectRule(document, identity)
  var keepLearn = decision === "allow"
    && !isTerminalIdentity(identity)
    && existing !== null
    && learns(existing.permissions)
  document.subjects = document.subjects.filter(function(item) {
    return compareIdentities(item.identity, identity) !== 0
  })
  if (decision !== null) {
    document.subjects.push({
      identity: identity,
      permissions: {
        context_read: decision,
        display: decision,
        suggest: decision,
        learn: keepLearn ? "allow" : "block",
        retention: keepLearn
          ? {mode: "bounded", days: existing.permissions.retention.days}
          : {mode: "none"}
      }
    })
  }
  document.subjects.sort(function(left, right) { return compareIdentities(left.identity, right.identity) })
}

// Style learning for one allowed non-terminal app or listed site (`days` 1..90
// enables, `null` disables and purges its kept samples on reconcile). Terminals
// (`com.mitchellh.ghostty`, `foot`, `org.codeberg.dnkl.foot`, `shell`/`bash`)
// are refused without mutating `document`.
function setSubjectLearning(document, identity, days, supportedApps) {
  if (!isObject(document) || !Array.isArray(document.subjects) || !isIdentity(identity)) return false
  if (isTerminalIdentity(identity)) return false
  var existing = subjectRule(document, identity)
  if (days === null) {
    if (!existing) return true
    existing.permissions = {
      context_read: existing.permissions.context_read,
      display: existing.permissions.display,
      suggest: existing.permissions.suggest,
      learn: "block",
      retention: {mode: "none"}
    }
    return true
  }
  if (!Number.isSafeInteger(days) || days < 1 || days > 90) return false
  if (identity.kind === "linux_app" && identity.adapter === "libreoffice") return false
  if (existing) {
    if (!allows(existing.permissions)) return false
    existing.permissions = {
      context_read: "allow",
      display: "allow",
      suggest: "allow",
      learn: "allow",
      retention: {mode: "bounded", days: days}
    }
    return true
  }
  var apps = Array.isArray(supportedApps) ? supportedApps : []
  var knownBlocklistApp = document.all_linux_apps === true && apps.some(function(app) {
    return !app.manual && compareIdentities(appIdentity(app.appId), identity) === 0
  })
  if (identity.kind !== "linux_app"
      || !knownBlocklistApp
      || document.subjects.length >= MAX_SUBJECTS) return false
  document.subjects.push({
    identity: identity,
    permissions: {
      context_read: "allow",
      display: "allow",
      suggest: "allow",
      learn: "allow",
      retention: {mode: "bounded", days: days}
    }
  })
  document.subjects.sort(function(left, right) { return compareIdentities(left.identity, right.identity) })
  return true
}

// Allowed non-terminal apps and listed websites that can opt into style
// learning; terminals and shells are never offered.
function styleTargets(document, supportedApps) {
  var apps = Array.isArray(supportedApps) ? supportedApps : []
  var subjects = isObject(document) && Array.isArray(document.subjects) ? document.subjects : []
  var targets = []
  var seenApps = {}
  apps.forEach(function(app) {
    var identity = appIdentity(app.appId)
    if (isTerminalIdentity(identity) || identity.adapter === "libreoffice") return
    seenApps[identity.adapter + ":" + app.appId] = true
    var rule = subjectRule(document, identity)
    if (!appEnabled(document, apps, app.appId) && !(rule && learns(rule.permissions))) return
    targets.push({
      key: "style_app_" + app.appId,
      kind: "app",
      appId: app.appId,
      identity: identity,
      label: app.name,
      learning: rule !== null && learns(rule.permissions),
      days: rule !== null ? learningDays(rule.permissions) : DEFAULT_RETENTION_DAYS
    })
  })
  subjects.forEach(function(entry) {
    if (!isObject(entry) || !isLinuxIdentity(entry.identity)) return
    if (isTerminalIdentity(entry.identity)) return
    if (entry.identity.adapter === "libreoffice" && !learns(entry.permissions)) return
    var token = entry.identity.adapter + ":" + entry.identity.app_id
    if (seenApps[token] || !allows(entry.permissions)) return
    seenApps[token] = true
    targets.push({
      key: "style_app_" + entry.identity.app_id,
      kind: "app",
      appId: entry.identity.app_id,
      identity: entry.identity,
      label: identityLabel(entry.identity, apps),
      learning: learns(entry.permissions),
      days: learningDays(entry.permissions)
    })
  })
  siteRules(document).forEach(function(rule) {
    if (!rule.allowed) return
    targets.push({
      key: "style_site_" + rule.origin,
      kind: "site",
      origin: rule.origin,
      identity: rule.identity,
      label: rule.origin,
      learning: rule.learning,
      days: rule.days
    })
  })
  return targets
}

function canonicalOverrideAppId(appId) {
  return appId === "chromium-browser" ? "chromium" : appId
}

function languageOverride(document, appId) {
  var overrides = objectValue(document, "language_overrides")
  var canonical = canonicalOverrideAppId(appId)
  var code = overrides[appId] || overrides[canonical]
    || (canonical === "chromium" ? overrides["chromium-browser"] : "")
  return WRITING_ORDER.indexOf(code) >= 0 ? code : ""
}

// Sets one app's writing-language override to a European writing language, or
// clears it (`"auto"`, `""`, `"reset"`), omitting `language_overrides` when
// empty. Persian (`"fa"`) is never set from the panel, while other apps'
// terminal-set `"fa"` entries stay untouched.
function setLanguageOverride(document, appId, code) {
  if (!isObject(document)) return false
  var canonical = canonicalOverrideAppId(appId)
  if (!isLinuxAppId(canonical)) return false
  var clear = code === "auto" || code === "" || code === "reset" || code === null
  if (!clear && PANEL_WRITING_ORDER.indexOf(code) < 0) return false
  var current = objectValue(document, "language_overrides")
  var next = {}
  Object.keys(current).forEach(function(key) {
    if (key !== canonical && !(canonical === "chromium" && key === "chromium-browser"))
      next[key] = current[key]
  })
  if (!clear) {
    if (Object.keys(next).length >= MAX_SUBJECTS) return false
    next[canonical] = code
  }
  if (Object.keys(next).length === 0) delete document.language_overrides
  else document.language_overrides = next
  return true
}

// Apps shown under App languages: browsers when websites are enabled or an
// override exists, enabled supported writing apps, and any other overridden or
// allowed Linux app id.
function languageOverrideTargets(document, supportedApps, installedDesktopIds) {
  var apps = Array.isArray(supportedApps) ? supportedApps : []
  var desktopIds = isObject(installedDesktopIds) ? installedDesktopIds : ({})
  var sitesOn = anySiteEnabled(document)
  var targets = []
  var seen = {}
  function add(appId, label) {
    var canonical = canonicalOverrideAppId(appId)
    if (!isLinuxAppId(canonical) || seen[canonical]) return
    seen[canonical] = true
    targets.push({
      key: "app_language_" + canonical,
      appId: canonical,
      label: label,
      override: languageOverride(document, canonical)
    })
  }
  BROWSERS.forEach(function(browser) {
    var hasOverride = languageOverride(document, browser.appId) !== ""
    var hasRule = appRule(document, browser.appId) !== null
    var installed = Array.isArray(browser.desktop)
      && browser.desktop.some(function(id) { return desktopIds[id] === true })
    if (hasOverride || hasRule || (sitesOn && (!browser.optional || installed)))
      add(browser.appId, browser.name)
  })
  apps.forEach(function(app) {
    var hasOverride = languageOverride(document, app.appId) !== ""
    if (app.appId === "libreoffice-writer" && !hasOverride) return
    if (appEnabled(document, apps, app.appId) || hasOverride)
      add(app.appId, app.name)
  })
  var subjects = isObject(document) && Array.isArray(document.subjects) ? document.subjects : []
  subjects.forEach(function(entry) {
    if (!isObject(entry) || !isLinuxIdentity(entry.identity)) return
    if (entry.identity.adapter === "libreoffice" && languageOverride(document, entry.identity.app_id) === "") return
    if (allows(entry.permissions))
      add(entry.identity.app_id, identityLabel(entry.identity, apps))
  })
  Object.keys(objectValue(document, "language_overrides")).sort().forEach(function(appId) {
    var canonical = canonicalOverrideAppId(appId)
    add(canonical, identityLabel(appIdentity(canonical), apps))
  })
  return targets
}
