.pragma library
// The popout's usage history, free of QML so it is tested without a desktop
// (tests/stats-model.test.mjs): the broker's content-free `stats.status`
// summary (badi stats) shaped into the week's bars and the leading apps.

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function count(parent, key) {
  return isObject(parent) && Number.isSafeInteger(parent[key]) && parent[key] >= 0 ? parent[key] : 0
}

// Shown, accepted and words saved over `window`: today, last_7_days or
// retained (90 days).
function totals(summary, window) {
  var counts = isObject(summary) && isObject(summary.totals) ? summary.totals[window] : null
  return {shown: count(counts, "shown"), accepted: count(counts, "accepted"), words: count(counts, "words")}
}

// Words saved on each of the seven days that end with the summary's today,
// oldest first; days without counts are zero.
function week(summary) {
  var today = isObject(summary) && typeof summary.today === "string"
    ? /^([0-9]{4})-([0-9]{2})-([0-9]{2})$/.exec(summary.today) : null
  if (!today) return []
  var byDay = {}
  var days = Array.isArray(summary.days) ? summary.days : []
  for (var index = 0; index < days.length; index++)
    if (isObject(days[index]) && typeof days[index].day === "string") byDay[days[index].day] = count(days[index], "words")
  var list = []
  for (var offset = 6; offset >= 0; offset--) {
    // Calendar arithmetic in UTC: the broker already chose the local date.
    var day = new Date(Date.UTC(Number(today[1]), Number(today[2]) - 1, Number(today[3]) - offset))
      .toISOString().slice(0, 10)
    list.push({day: day, words: byDay[day] || 0})
  }
  return list
}

// An app's name from `names` (app id to name), else its id; a site's host
// and any port that is not its scheme's default.
function label(identity, names) {
  if (!isObject(identity)) return ""
  if (identity.kind === "linux_app") {
    var app = String(identity.app_id)
    return isObject(names) && typeof names[app] === "string" ? names[app] : app
  }
  var standard = identity.scheme === "https" ? 443 : 80
  return String(identity.host) + (identity.port === standard ? "" : ":" + identity.port)
}

// The `limit` apps and sites with the most words saved in the last seven
// days, then the most shown, each with its label.
function leaders(summary, names, limit) {
  var subjects = isObject(summary) && Array.isArray(summary.subjects) ? summary.subjects : []
  return subjects
    .filter(function(subject) { return isObject(subject) && isObject(subject.last_7_days) })
    .map(function(subject) {
      var counts = subject.last_7_days
      return {label: label(subject.identity, names), words: count(counts, "words"), shown: count(counts, "shown"),
        accepted: count(counts, "accepted")}
    })
    .filter(function(entry) { return entry.shown + entry.accepted > 0 })
    .sort(function(left, right) { return right.words - left.words || right.shown - left.shown })
    .slice(0, limit)
}
