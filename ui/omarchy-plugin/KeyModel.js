.pragma library
// The settings popout's keyboard model, free of QML so it is tested without a
// desktop (tests/key-model.test.mjs): one cursor over the controller's typed
// rows, moved with j/k, a choice stepped with h/l, Enter and x, and search.

// The cursor after j (dy 1) or k (dy -1); -1 is no row yet, and the first
// move lands on the first row.
function step(cursor, count, dy) {
  if (count === 0) return cursor
  return cursor < 0 ? 0 : Math.max(0, Math.min(count - 1, cursor + dy))
}

// The value h (dx -1) or l (dx 1) picks on an enabled choice row; null when
// the row is no choice or the step would leave its options. With nothing
// chosen yet, l picks the first option.
function choice(row, dx) {
  if (!row || row.kind !== "choice" || !row.enabled) return null
  var values = row.options.map(function(option) { return option.value })
  var next = values.indexOf(row.value) + dx
  return next >= 0 && next < values.length ? values[next] : null
}

// What Enter does on a row: "focus" its text field, "activate" it, or null.
function action(row) {
  if (!row || !row.enabled) return null
  if (row.kind === "field") return "focus"
  return row.activate ? "activate" : null
}

// Whether x removes the row: enabled site rules only.
function removable(row) {
  return Boolean(row && row.kind === "site" && row.enabled)
}

// The rows a search shows: every row for an empty query, otherwise those
// whose label, detail or section contains it, ignoring case.
function filter(rows, query) {
  var needle = String(query || "").trim().toLowerCase()
  if (!needle) return rows
  return rows.filter(function(row) {
    return (row.label + " " + row.detail + " " + row.section).toLowerCase().indexOf(needle) >= 0
  })
}
