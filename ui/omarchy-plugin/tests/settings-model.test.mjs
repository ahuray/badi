// Pure settings validation, status ladder and policy mutations
// (SettingsModel.js), loaded as the QML engine loads a `.pragma library` file.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../SettingsModel.js', import.meta.url), 'utf8');
assert.match(source, /^\.pragma library\n/u);
const SettingsModel = vm.runInNewContext(`${source.replace(/^\.pragma library\n/u, '')}
({
  WRITING_ORDER, PANEL_WRITING_ORDER, SPELLING_ORDER, DEFAULT_RETENTION_DAYS, MAX_SUBJECTS, BROWSERS,
  isObject, objectValue, hasBoolean, boolValue, integerValue,
  isIdentity, isTerminalIdentity, compareIdentities, isOptionalSetting, isSettingsDocument,
  status, appAdapter, appIdentity, originText, siteIdentity, identityLabel,
  allows, learns, learningDays, subjectRule, appRule, appEnabled, siteRules, anySiteEnabled, anyAppEnabled,
  withLanguage, putSpelling, putWriting, putRule,
  setSubjectLearning, styleTargets,
  canonicalOverrideAppId, languageOverride, setLanguageOverride, languageOverrideTargets
})`);

const plain = value => JSON.parse(JSON.stringify(value));

const supportedApps = [
  { name: 'Omawrite', appId: 'omawrite', context: 'learned' },
  { name: 'Ghostty', appId: 'com.mitchellh.ghostty', context: 'line', manual: true },
  { name: 'foot', appId: 'foot', context: 'line', manual: true },
  { name: 'Xournal++', appId: 'com.github.xournalpp.xournalpp', context: 'learned', manual: true },
  { name: 'Obsidian', appId: 'obsidian', context: 'learned' },
  { name: 'Neovim', appId: 'nvim', context: 'learned' },
  { name: 'Bash', appId: 'bash', context: 'line' },
  { name: 'LibreOffice Writer', appId: 'libreoffice', context: 'learned' },
  { name: 'Writer spelling', appId: 'libreoffice-writer' },
  { name: 'Telegram', appId: 'telegram', context: 'learned' },
];

const baseSettings = () => ({
  schema: 'badi.settings.v2',
  revision: 4,
  paused: false,
  subjects: [
    {
      identity: { kind: 'browser_origin', adapter: 'chromium', scheme: 'https', host: 'mail.example.com', port: 443 },
      permissions: { suggest: 'allow', display: 'allow', context_read: 'allow', learn: 'block', retention: { mode: 'none' } },
    },
    {
      identity: { kind: 'linux_app', adapter: 'fcitx', app_id: 'omawrite' },
      permissions: { suggest: 'allow', display: 'allow', context_read: 'allow', learn: 'block', retention: { mode: 'none' } },
    },
  ],
});

test('status ladder resolves every rung in priority order without drift between bar and hero', () => {
  const readyBase = {
    brokerReachable: true,
    brokerPaused: false,
    controlPlaneDegraded: false,
    settingsPaused: false,
    sharingHeld: false,
    anyAppEnabled: true,
    serviceActive: 'active',
  };

  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, brokerReachable: false, serviceActive: 'active' })), {
    kind: 'loading',
    ready: false,
    paused: false,
    loading: true,
    label: 'Model starting or unavailable',
    mode: { glyph: '◌', text: 'Loading', tone: 'dim' },
  });
  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, brokerReachable: false, serviceActive: 'activating' })), {
    kind: 'loading',
    ready: false,
    paused: false,
    loading: true,
    label: 'Model starting or unavailable',
    mode: { glyph: '◌', text: 'Loading', tone: 'dim' },
  });
  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, brokerReachable: false, serviceActive: 'inactive' })), {
    kind: 'offline',
    ready: false,
    paused: false,
    loading: false,
    label: 'Model offline',
    mode: { glyph: '○', text: 'Model offline', tone: 'dim' },
  });
  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, controlPlaneDegraded: true, brokerPaused: true })), {
    kind: 'degraded',
    ready: false,
    paused: true,
    loading: false,
    label: 'Settings need attention',
    mode: { glyph: '!', text: 'Settings need attention', tone: 'urgent' },
  });
  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, brokerPaused: true, sharingHeld: true, settingsPaused: false })), {
    kind: 'held',
    ready: false,
    paused: true,
    loading: false,
    label: 'Held while the screen is shared',
    mode: { glyph: '⏸', text: 'Held · screen shared', tone: 'dim' },
  });
  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, brokerPaused: true, sharingHeld: true, settingsPaused: true })), {
    kind: 'paused',
    ready: false,
    paused: true,
    loading: false,
    label: 'Predictions paused',
    mode: { glyph: '⏸', text: 'Paused', tone: 'dim' },
  }, 'an explicit user pause stays Paused even while the screen is shared');
  assert.deepEqual(plain(SettingsModel.status({ ...readyBase, anyAppEnabled: false })), {
    kind: 'no_apps',
    ready: false,
    paused: false,
    loading: false,
    label: 'Applications disabled',
    mode: { glyph: '◇', text: 'No app allowed', tone: 'dim' },
  });
  assert.deepEqual(plain(SettingsModel.status(readyBase)), {
    kind: 'ready',
    ready: true,
    paused: false,
    loading: false,
    label: 'Model ready',
    mode: { glyph: '⏵⏵', text: 'Suggesting', tone: 'accent' },
  });
});

test('settings validator covers every optional schema key and rejects unknown keys', () => {
  const schema = JSON.parse(readFileSync(
    new URL('../../../broker/schemas/badi.settings.v2.schema.json', import.meta.url), 'utf8'));
  const optionalKeys = Object.keys(schema.properties)
    .filter(key => !schema.required.includes(key))
    .sort();
  const sampleValues = {
    all_linux_apps: true,
    all_web_origins: true,
    battery_mode: 'quiet',
    context_words: true,
    context_words_off: [{ kind: 'linux_app', adapter: 'fcitx', app_id: 'omawrite' }],
    escape_quiet_seconds: 60,
    language_overrides: { 'brave-origin': 'de' },
    languages_chosen: true,
    mid_line: true,
    places_chosen: true,
    screen_share: 'hold',
    spelling_languages: ['en', 'de'],
    style_learning: true,
    suggestion_length: 'short',
    trailing_space_healing: { en: true },
    writing_languages: ['en', 'de'],
    writing_model: 'gemma-3-1b',
  };
  assert.deepEqual(Object.keys(sampleValues).sort(), optionalKeys);
  for (const [key, value] of Object.entries(sampleValues)) {
    assert.equal(SettingsModel.isSettingsDocument({ ...baseSettings(), [key]: value }), true, `refused ${key}`);
  }
  assert.equal(SettingsModel.isSettingsDocument({ ...baseSettings(), future_key: true }), false);
});

test('terminals and shells never appear in styleTargets and cannot enable style learning', () => {
  const terminals = [
    { kind: 'linux_app', adapter: 'fcitx', app_id: 'com.mitchellh.ghostty' },
    { kind: 'linux_app', adapter: 'fcitx', app_id: 'foot' },
    { kind: 'linux_app', adapter: 'fcitx', app_id: 'org.codeberg.dnkl.foot' },
    { kind: 'linux_app', adapter: 'shell', app_id: 'bash' },
  ];
  const doc = baseSettings();
  doc.all_linux_apps = true;
  doc.style_learning = true;
  for (const identity of terminals) {
    assert.equal(SettingsModel.isTerminalIdentity(identity), true);
    SettingsModel.putRule(doc, identity, 'allow');
  }
  SettingsModel.putRule(doc, SettingsModel.appIdentity('libreoffice-writer'), 'allow');
  assert.equal(SettingsModel.isSettingsDocument(doc), true);

  const snapshot = JSON.stringify(doc);
  for (const identity of terminals) {
    assert.equal(SettingsModel.setSubjectLearning(doc, identity, 30, supportedApps), false);
    assert.equal(JSON.stringify(doc), snapshot, `mutated document for ${identity.app_id}`);
  }
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.appIdentity('libreoffice-writer'), 30, supportedApps), false);

  const targets = plain(SettingsModel.styleTargets(doc, supportedApps));
  const keys = targets.map(target => target.key);
  for (const forbidden of [
    'style_app_com.mitchellh.ghostty',
    'style_app_foot',
    'style_app_org.codeberg.dnkl.foot',
    'style_app_bash',
    'style_app_libreoffice-writer',
  ]) {
    assert.equal(keys.includes(forbidden), false, `unexpected style target ${forbidden}`);
  }
  assert.deepEqual(keys, [
    'style_app_omawrite',
    'style_app_obsidian',
    'style_app_nvim',
    'style_app_libreoffice',
    'style_app_telegram',
    'style_site_https://mail.example.com',
  ]);
});

test('style learning mutations bind to allowed non-terminal subjects and blocking revokes learning', () => {
  const doc = baseSettings();
  doc.style_learning = true;

  // Enabling learning on an allowed app and an allowed site sets bounded retention.
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.appIdentity('omawrite'), 30, supportedApps), true);
  const mailSite = SettingsModel.siteIdentity('https://mail.example.com');
  assert.equal(SettingsModel.setSubjectLearning(doc, mailSite, 14, supportedApps), true);
  assert.equal(SettingsModel.isSettingsDocument(doc), true);

  let targets = plain(SettingsModel.styleTargets(doc, supportedApps));
  assert.deepEqual(targets.map(({ key, learning, days }) => [key, learning, days]), [
    ['style_app_omawrite', true, 30],
    ['style_site_https://mail.example.com', true, 14],
  ]);

  // Disallowed apps and unlisted sites cannot enable style learning.
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.appIdentity('telegram'), 30, supportedApps), false);
  doc.all_web_origins = true;
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.siteIdentity('https://other.example.com'), 30, supportedApps), false);

  // In app blocklist mode, enabling learning on a non-manual app creates its rule;
  // manual or unlisted apps without an explicit allow rule stay refused.
  doc.all_linux_apps = true;
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.appIdentity('com.github.xournalpp.xournalpp'), 30, supportedApps), false);
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.appIdentity('unlisted.app'), 30, supportedApps), false);
  assert.equal(SettingsModel.setSubjectLearning(
    doc, { kind: 'linux_app', adapter: 'obsidian', app_id: 'telegram' }, 30, supportedApps), false);
  assert.equal(SettingsModel.setSubjectLearning(
    doc, SettingsModel.appIdentity('telegram'), 30, supportedApps), true);
  assert.equal(SettingsModel.isSettingsDocument(doc), true);
  assert.equal(SettingsModel.learns(SettingsModel.appRule(doc, 'telegram').permissions), true);

  // Keeping an allowed rule preserves custom retention; blocking revokes learning.
  SettingsModel.putRule(doc, mailSite, 'allow');
  assert.equal(SettingsModel.learningDays(SettingsModel.subjectRule(doc, mailSite).permissions), 14);
  SettingsModel.putRule(doc, SettingsModel.appIdentity('omawrite'), 'block');
  assert.equal(SettingsModel.learns(SettingsModel.appRule(doc, 'omawrite').permissions), false);
  assert.deepEqual(plain(SettingsModel.appRule(doc, 'omawrite').permissions.retention), { mode: 'none' });

  // Turning learning off on a site keeps suggestions allowed and resets retention.
  assert.equal(SettingsModel.setSubjectLearning(doc, mailSite, null, supportedApps), true);
  const mailPerms = plain(SettingsModel.subjectRule(doc, mailSite).permissions);
  assert.deepEqual(mailPerms, {
    suggest: 'allow',
    display: 'allow',
    context_read: 'allow',
    learn: 'block',
    retention: { mode: 'none' },
  });
  assert.equal(SettingsModel.isSettingsDocument(doc), true);
});

test('per-app language overrides set European languages, preserve terminal Persian, canonicalize Chromium and omit empty maps', () => {
  const doc = baseSettings();
  doc.language_overrides = { 'chromium-browser': 'fr', telegram: 'fa' };

  assert.equal(SettingsModel.languageOverride(doc, 'chromium'), 'fr');
  assert.equal(SettingsModel.languageOverride(doc, 'telegram'), 'fa');
  assert.equal(SettingsModel.setLanguageOverride(doc, 'omawrite', 'fa'), false, 'Persian stays terminal-only');

  // Updating Chromium canonicalizes chromium-browser -> chromium and preserves Telegram's Persian.
  assert.equal(SettingsModel.setLanguageOverride(doc, 'chromium-browser', 'de'), true);
  assert.deepEqual(plain(doc.language_overrides), { chromium: 'de', telegram: 'fa' });
  assert.equal(SettingsModel.isSettingsDocument(doc), true);

  //Targets include browsers when a site is enabled, allowed apps, and overridden disabled apps.
  const targets = plain(SettingsModel.languageOverrideTargets(
    doc, supportedApps, { 'app.zen_browser.twilight': true }));
  assert.deepEqual(targets.map(({ key, label, override }) => [key, label, override]), [
    ['app_language_chromium', 'Chromium', 'de'],
    ['app_language_brave-origin', 'Brave', ''],
    ['app_language_zen', 'Zen', ''],
    ['app_language_zen-twilight', 'Zen Twilight', ''],
    ['app_language_omawrite', 'Omawrite', ''],
    ['app_language_telegram', 'Telegram', 'fa'],
  ]);

  // Resetting every override removes language_overrides completely, including terminal Persian when chosen.
  assert.equal(SettingsModel.setLanguageOverride(doc, 'chromium', 'auto'), true);
  assert.deepEqual(plain(doc.language_overrides), { telegram: 'fa' });
  assert.equal(SettingsModel.setLanguageOverride(doc, 'telegram', 'auto'), true);
  assert.equal('language_overrides' in doc, false);
  assert.equal(SettingsModel.isSettingsDocument(doc), true);
});

test('siteIdentity and originText round-trip canonical DNS, IPv4 and bracketed IPv6 origins', () => {
  const loopback6 = SettingsModel.siteIdentity('http://[0:0:0:0:0:0:0:1]:8080/');
  assert.deepEqual(plain(loopback6), {
    kind: 'browser_origin',
    adapter: 'chromium',
    scheme: 'http',
    host: '::1',
    port: 8080,
  });
  assert.equal(SettingsModel.originText(loopback6), 'http://[::1]:8080');
  assert.deepEqual(plain(SettingsModel.siteIdentity(SettingsModel.originText(loopback6))), plain(loopback6));

  const doc6 = SettingsModel.siteIdentity('https://[2001:0db8:0:0:1:0:0:1]');
  assert.equal(doc6.host, '2001:db8::1:0:0:1');
  assert.equal(SettingsModel.originText(doc6), 'https://[2001:db8::1:0:0:1]');

  const mapped = SettingsModel.siteIdentity('http://[::ffff:127.0.0.1]:3000');
  assert.equal(mapped.host, '::ffff:127.0.0.1');
  assert.equal(SettingsModel.originText(mapped), 'http://[::ffff:127.0.0.1]:3000');

  const ipv4 = SettingsModel.siteIdentity('http://127.0.0.1:8080');
  assert.equal(ipv4.host, '127.0.0.1');
  assert.equal(SettingsModel.originText(ipv4), 'http://127.0.0.1:8080');

  for (const invalid of [
    'https://::1:8080',
    'https://[2001:db8:::1]',
    'https://[2001:db8:1]',
    'https://..',
    'https://12345',
    'https://0x7f000001',
    'https://-bad.example.com',
    'https://example.com.',
  ]) {
    assert.equal(SettingsModel.siteIdentity(invalid), null, `accepted invalid origin ${invalid}`);
  }
});
