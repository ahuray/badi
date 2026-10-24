.pragma library
// The settings popout's keyboard model, free of QML so it is tested without a
// desktop (tests/key-model.test.mjs): one cursor over the controller's typed
// rows, moved with j/k, a choice stepped with h/l, a multi-select row's
// chips walked with h/l and toggled with Enter, x, search, and the cursor
// staying on its row while the rows change.

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

// The chip h (dx -1) or l (dx 1) moves to on an enabled multi-select row,
// clamped to its options; null on any other row.
function chip(row, index, dx) {
  if (!row || row.kind !== "multi" || !row.enabled || row.options.length === 0) return null
  var from = index < 0 ? 0 : index
  return Math.max(0, Math.min(row.options.length - 1, from + dx))
}

// What Enter does on a row: "focus" its text field, "toggle" the chip under
// the cursor of a multi-select row, "activate" it, or null.
function action(row) {
  if (!row || !row.enabled) return null
  if (row.kind === "field") return "focus"
  if (row.kind === "multi") return row.toggle ? "toggle" : null
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

// Where the cursor goes when the rows change: onto the row it was on (by
// key), wherever that row now is; else the same place, clamped to the rows.
// -1 stays -1, no row yet.
function follow(rows, key, cursor) {
  if (cursor < 0) return -1
  for (var index = 0; index < rows.length; index += 1) {
    if (key !== "" && rows[index].key === key) return index
  }
  return Math.min(cursor, rows.length - 1)
}

// Whether two row lists show exactly the same thing: every field but the
// actions. The rows' actions read the state those fields show, so a list
// that shows the same may keep its delegates, and a text field its draft.
function sameRows(rows, other) {
  return JSON.stringify(rows) === JSON.stringify(other)
}
