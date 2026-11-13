-- Badi for Neovim: the editor owns acquisition, the inline preview and the
-- edit. A suggestion is bound to the window, buffer, changedtick and caret it
-- was requested for; every mismatch withdraws it, and an acceptance is
-- revalidated before and after the broker's one-shot grant.
local context = require('badi.context')
local Bridge = require('badi.bridge')
local revision = require('badi.revision')

local api = vim.api
local M = {}

local defaults = {
  -- Prose filetypes; any other buffer gets no suggestions.
  filetypes = { 'markdown', 'text', 'gitcommit', 'mail', 'rst', 'tex', 'asciidoc', 'org', 'norg' },
  -- Ask on its own after a typing pause; false leaves only explicit requests.
  automatic = true,
  delay_ms = 120,
  -- 'en', 'de' or 'fa'; nil uses $BADI_LANGUAGE, then the locale.
  language = nil,
  keys = { accept = '<C-Right>', accept_word = '<Tab>', request = '<C-S-Space>' },
  node = 'node',
  -- Defaults to $BADI_EDITOR_DIR (or ~/.local/lib/badi/editors)/neovim/bridge.mjs.
  bridge = nil,
}

-- After a key that typed the shown suggestion's next characters, the broker
-- can answer at once from its carry (ADR 0004).
local TYPE_THROUGH_DELAY_MS = 20
local namespace = api.nvim_create_namespace('badi')

local state = {
  config = nil,
  bridge = nil,
  generation = 0,
  timer = nil,
  -- The armed automatic request: { buf, tick, row, col }.
  scheduled = nil,
  -- The request in flight: { generation, snapshot }.
  pending = nil,
  -- The preview: { snapshot, text, display, replace_before, mark, saved }.
  visible = nil,
  accepting = false,
  -- Text-only memory of the last preview, for type-through detection.
  shown = nil,
  focused = true,
  -- Neovim holds its terminal's focus, as the terminal reported it. While
  -- it does, the broker keeps the input method's typed line out of the
  -- terminal, so these keys get one suggester (ADR 0009).
  holds_terminal = false,
  -- The broker's policy for Neovim as the bridge last reported it while
  -- connected: { allowed, automatic }, or nil until then. No buffer text is
  -- read for a request it does not allow.
  policy = nil,
  status = 'idle',
}

local function set_status(reason)
  if type(reason) == 'string' and reason:match('^[a-z_]+$') then state.status = reason end
end

--- Keeps the policy from a bridge report. Only a connected bridge's report
--- decides; while offline the next request asks again, which reconnects.
local function set_policy(report)
  if report.connected == true and type(report.allowed) == 'boolean' then
    state.policy = { allowed = report.allowed, automatic = report.allowed and report.automatic == true }
  else
    state.policy = nil
  end
end

--- Why the known policy refuses a request, or nil when it allows it.
local function policy_reason(explicit)
  if not state.policy.allowed then return 'app_disabled_or_model_paused' end
  if not explicit and not state.policy.automatic then return 'manual_required' end
  return nil
end

local function bridge_command()
  local path = state.config.bridge
  if not path then
    local root = vim.env.BADI_EDITOR_DIR or ((vim.env.HOME or '') .. '/.local/lib/badi/editors')
    path = root .. '/neovim/bridge.mjs'
  end
  return { state.config.node, path }
end

local function facts()
  return {
    mode = api.nvim_get_mode().mode,
    pumvisible = vim.fn.pumvisible() ~= 0,
    executing = vim.fn.reg_executing() ~= '',
    focused = state.focused,
  }
end

-- InsertEnter runs before the mode changes; it asks as if already inserting.
local function blocked_reason(buf, assume_insert, selection)
  local current = facts()
  if assume_insert then current.mode = 'i' end
  return context.blocked_reason(buf or api.nvim_get_current_buf(), state.config, current, selection)
end

local function snapshot()
  local win = api.nvim_get_current_win()
  local buf = api.nvim_win_get_buf(win)
  local cursor = api.nvim_win_get_cursor(win)
  local row, col = cursor[1] - 1, cursor[2]
  return {
    win = win,
    buf = buf,
    tick = api.nvim_buf_get_changedtick(buf),
    row = row,
    col = col,
    line = api.nvim_buf_get_lines(buf, row, row + 1, true)[1],
  }
end

--- The editor is still exactly where `s` was taken, and may still be read.
local function matches(s)
  if not s or not api.nvim_win_is_valid(s.win) or not api.nvim_buf_is_valid(s.buf) then return false end
  if api.nvim_get_current_win() ~= s.win or api.nvim_win_get_buf(s.win) ~= s.buf then return false end
  if api.nvim_buf_get_changedtick(s.buf) ~= s.tick then return false end
  if s.kind == 'revision' then
    return blocked_reason(s.buf, false, true) == nil and revision.matches(s)
  end
  local cursor = api.nvim_win_get_cursor(s.win)
  return cursor[1] == s.row + 1 and cursor[2] == s.col and blocked_reason(s.buf) == nil
end

local function stop_timer()
  if state.timer then
    state.timer:stop()
    state.timer:close()
    state.timer = nil
  end
  state.scheduled = nil
end

local function remove_maps(visible)
  local mode = visible.map_mode or 'i'
  for lhs, saved in pairs(visible.saved) do
    pcall(vim.keymap.del, mode, lhs, { buffer = visible.snapshot.buf })
    if saved then pcall(vim.fn.mapset, saved.mode or mode, false, saved) end
  end
  visible.saved = {}
end

local function clear_preview()
  local visible = state.visible
  if not visible then return end
  state.visible = nil
  if visible.preview_win and api.nvim_win_is_valid(visible.preview_win) then
    pcall(api.nvim_win_close, visible.preview_win, true)
  end
  if api.nvim_buf_is_valid(visible.snapshot.buf) then
    if visible.mark then pcall(api.nvim_buf_del_extmark, visible.snapshot.buf, namespace, visible.mark) end
    remove_maps(visible)
  end
end

--- The preview's chunks: what Tab takes a step brighter than the rest.
local function chunks(text, word)
  if type(word) == 'string' and #word > 0 and #word < #text and text:sub(1, #word) == word then
    return { { word, 'BadiSuggestionWord' }, { text:sub(#word + 1), 'BadiSuggestion' } }
  end
  return { { text, 'BadiSuggestion' } }
end

--- The broker streamed a longer version of the shown continuation: redraw
--- the same extmark with it.
local function extend(text, word)
  local visible = state.visible
  if not visible or not visible.text or visible.replace_before ~= nil or state.accepting
      or type(text) ~= 'string' or not matches(visible.snapshot)
      or not context.insertable(text) or text:sub(1, #visible.text) ~= visible.text then return end
  local s = visible.snapshot
  local ok = pcall(api.nvim_buf_set_extmark, s.buf, namespace, s.row, s.col, {
    id = visible.mark, virt_text = chunks(text, word), virt_text_pos = 'inline', hl_mode = 'combine',
  })
  if not ok then return end
  visible.text = text
  visible.display = text
  state.shown.text = text
end

--- Withdraws the preview and any request in flight. `notify` names the
--- bridge operation that releases the broker's session: 'cancel' keeps the
--- type-through carry, 'dismiss' ends it.
local function invalidate(notify)
  local active = state.pending ~= nil or state.visible ~= nil or state.accepting or state.revision_cycle ~= nil
  state.generation = state.generation + 1
  state.pending = nil
  state.revision_cycle = nil
  state.accepting = false
  stop_timer()
  clear_preview()
  if active and notify and state.bridge then state.bridge:notify({ op = notify }) end
end

local function ensure_bridge()
  if state.bridge and state.bridge:running() then return true end
  state.bridge = state.bridge or Bridge.new({
    command = bridge_command(),
    on_event = function(message)
      if message.event == 'clear' then
        -- Expiry, revocation, pause or disconnect: the grant behind the
        -- preview is gone, so the preview goes too. This also stale-outs an
        -- acceptance whose grant reply has not been applied yet.
        if state.visible or state.accepting then invalidate(nil) end
      elseif message.event == 'update' then
        extend(message.text, message.word)
      elseif message.event == 'state' then
        set_status(message.state)
        set_policy(message)
        if state.policy and not state.policy.allowed then invalidate(nil) end
        if message.state == 'offline' or message.state == 'offline_retry' then invalidate(nil) end
      end
    end,
    on_exit = function()
      invalidate(nil)
      state.policy = nil
      set_status('bridge_exited')
    end,
  })
  if not state.bridge:start() then return false end
  -- A new bridge is a new broker connection, which holds nothing yet.
  if state.holds_terminal then state.bridge:notify({ op = 'focus', focused = true }) end
  return true
end

-- Only a terminal that reports focus changes (FocusGained) makes Neovim
-- claim it, so a terminal or multiplexer without focus events never leaves
-- a claim behind that no FocusLost would end.
local function hold_terminal(holds)
  if state.holds_terminal == holds then return end
  state.holds_terminal = holds
  if state.bridge and state.bridge:running() then
    state.bridge:notify({ op = 'focus', focused = holds })
  elseif holds then
    ensure_bridge()
  end
end

local accept
local pick

local function on_key(kind, lhs)
  local visible = state.visible
  if visible and state.accepting then return end
  if visible and matches(visible.snapshot) then
    accept(visible.snapshot.kind == 'revision' and 'all' or kind == 'accept_word' and 'word' or 'all')
    return
  end
  -- Not ours: withdraw the preview and its mappings, then hand the key to
  -- whatever the user or another plugin mapped, or to Neovim itself.
  invalidate('cancel')
  local remaining = vim.fn.maparg(lhs, visible and visible.map_mode or 'i', false, true)
  -- Never feed the key back to this mapping.
  if remaining.buffer == 1 and remaining.desc == 'Badi' then return end
  api.nvim_feedkeys(api.nvim_replace_termcodes(lhs, true, false, true), 'mt', false)
end

local function show_revision(s, reply)
  local width, height = api.nvim_win_get_width(s.win), api.nvim_win_get_height(s.win)
  if width < 16 or height < 4 then return false end
  local preview = revision.preview(s, reply, width - 2, math.floor(height / 2))
  if not preview then return false end
  -- Virtual lines below a bottom-row selection can be completely offscreen.
  -- Keep the entire marked preview inside the requesting window instead.
  local buf = api.nvim_create_buf(false, true)
  vim.bo[buf].bufhidden = 'wipe'
  local lines = {}
  for _, line in ipairs(preview.lines) do
    local parts = {}
    for _, chunk in ipairs(line) do parts[#parts + 1] = chunk[1] end
    lines[#lines + 1] = table.concat(parts)
  end
  api.nvim_buf_set_lines(buf, 0, -1, true, lines)
  local highlights = api.nvim_create_namespace('badi_revision_preview')
  for row, line in ipairs(preview.lines) do
    local col = 0
    for _, chunk in ipairs(line) do
      api.nvim_buf_set_extmark(buf, highlights, row - 1, col,
        { end_col = col + #chunk[1], hl_group = chunk[2] })
      col = col + #chunk[1]
    end
  end
  vim.bo[buf].modifiable = false
  local ok, win = pcall(api.nvim_open_win, buf, false, { relative = 'win', win = s.win, col = 0,
    row = math.max(0, math.min(vim.fn.winline(), height - #lines - 2)), width = width - 2, height = #lines,
    focusable = false, style = 'minimal', border = 'single', noautocmd = true, zindex = 200 })
  if not ok then api.nvim_buf_delete(buf, { force = true }); return false end
  vim.wo[win].wrap = false
  local visible = { snapshot = s, text = reply.text, display = preview.display, chunks = preview.chunks,
    preview_win = win, owner_width = width, owner_height = height,
    preview_width = width - 2, preview_height = #lines, saved = {}, map_mode = 'x' }
  state.visible, state.shown = visible, nil
  local lhs = state.config.keys.accept_word
  if lhs and lhs ~= '' then
    local previous = vim.fn.maparg(lhs, 'x', false, true)
    visible.saved[lhs] = previous.buffer == 1 and previous.desc ~= 'Badi' and previous or false
    vim.keymap.set('x', lhs, function() on_key('accept', lhs) end,
      { buffer = s.buf, nowait = true, silent = true, desc = 'Badi' })
  end
  set_status('revision')
  return true
end

local function on_pick_key(index, lhs)
  local visible = state.visible
  if visible and state.accepting then return end
  if visible and visible.list and matches(visible.snapshot) then
    pick(index)
    return
  end
  invalidate('cancel')
  local remaining = vim.fn.maparg(lhs, 'i', false, true)
  if remaining.buffer == 1 and remaining.desc == 'Badi' then return end
  api.nvim_feedkeys(api.nvim_replace_termcodes(lhs, true, false, true), 'mt', false)
end

local function show(s, text, word, replace_before)
  local display = text
  if replace_before ~= nil then
    if context.replacement_start(s.line, s.col, replace_before, text) == nil then return false end
    -- A correction is drawn whole: the original first, then the correction.
    display = ' ' .. vim.trim(replace_before) .. ' → ' .. vim.trim(text)
  end
  local ok, mark = pcall(api.nvim_buf_set_extmark, s.buf, namespace, s.row, s.col, {
    virt_text = chunks(display, replace_before == nil and word or ''),
    virt_text_pos = 'inline',
    hl_mode = 'combine',
  })
  if not ok then return false end
  -- Draw it only in the window it was requested for, not in other windows
  -- showing the same buffer (an experimental API; older builds draw in both).
  if api.nvim__ns_set then pcall(api.nvim__ns_set, namespace, { wins = { s.win } }) end
  local visible = { snapshot = s, text = text, display = display, replace_before = replace_before, mark = mark, saved = {} }
  state.visible = visible
  -- Only appendable text can carry through typing (ADR 0004).
  state.shown = replace_before == nil
    and { buf = s.buf, row = s.row, line_before = s.line:sub(1, s.col), text = text } or nil
  for kind, lhs in pairs({ accept = state.config.keys.accept, accept_word = state.config.keys.accept_word }) do
    if lhs and lhs ~= '' then
      local previous = vim.fn.maparg(lhs, 'i', false, true)
      -- Only a buffer-local mapping is replaced by ours; restore it afterwards.
      visible.saved[lhs] = (previous.buffer == 1 and previous.desc ~= 'Badi') and previous or false
      vim.keymap.set('i', lhs, function() on_key(kind, lhs) end,
        { buffer = s.buf, nowait = true, silent = true, desc = 'Badi' })
    end
  end
  set_status('suggestion')
  return true
end

--- Draws the numbered alternative words at the caret (ADR 0014). Only the
--- digits the list numbers are Badi's, and only while it is drawn. It
--- stands where the shown suggestion stood, its leading space included.
local function show_list(s, words, lead)
  if type(words) ~= 'table' or #words < 1 or #words > 4 then return false end
  local parts = {}
  for index, word in ipairs(words) do
    if type(word) ~= 'string' or not context.insertable(word) then return false end
    parts[#parts + 1] = index .. '.' .. word
  end
  local text = lead .. table.concat(parts, '  ')
  local ok, mark = pcall(api.nvim_buf_set_extmark, s.buf, namespace, s.row, s.col, {
    virt_text = { { text, 'BadiSuggestion' } },
    virt_text_pos = 'inline',
    hl_mode = 'combine',
  })
  if not ok then return false end
  if api.nvim__ns_set then pcall(api.nvim__ns_set, namespace, { wins = { s.win } }) end
  local visible = { snapshot = s, list = words, list_text = text, mark = mark, saved = {} }
  state.visible = visible
  state.shown = nil
  for index = 1, #words do
    local lhs = tostring(index)
    local previous = vim.fn.maparg(lhs, 'i', false, true)
    visible.saved[lhs] = (previous.buffer == 1 and previous.desc ~= 'Badi') and previous or false
    vim.keymap.set('i', lhs, function() on_pick_key(index, lhs) end,
      { buffer = s.buf, nowait = true, silent = true, desc = 'Badi' })
  end
  set_status('alternative_list')
  return true
end

--- A digit on a shown numbered list: the picked word's continuation
--- replaces the list, accepted like any suggestion.
pick = function(index)
  local visible = state.visible
  if not visible or not visible.list then return end
  local word = visible.list[index]
  if not word then return end
  local s = visible.snapshot
  local generation = state.generation
  state.pending = { generation = generation, snapshot = s }
  set_status('thinking')
  state.bridge:request({ op = 'pick', word = word }, function(reply)
    if state.generation ~= generation then return end
    state.pending = nil
    if not reply.ok or not context.insertable(reply.text) or not matches(s) then
      set_status(reply.ok and (reply.text == nil and 'no_suggestion' or 'stale') or reply.reason)
      invalidate('cancel')
      return
    end
    clear_preview()
    if not show(s, reply.text, reply.word) then
      set_status('stale')
      state.bridge:notify({ op = 'cancel' })
    end
  end)
end

--- Replaces the shown suggestion with one that starts with another word. It
--- stays drawn until the answer arrives; no answer withdraws it.
local function alternative()
  local visible = state.visible
  local s = visible.snapshot
  local lead = visible.text and visible.text:sub(1, 1) == ' ' and ' ' or ''
  local generation = state.generation
  state.pending = { generation = generation, snapshot = s }
  set_status('thinking')
  state.bridge:request({ op = 'alternative' }, function(reply)
    if state.generation ~= generation then return end
    state.pending = nil
    if reply.ok and reply.words and matches(s) then
      clear_preview()
      if show_list(s, reply.words, lead) then return end
      set_status('stale')
      state.bridge:notify({ op = 'cancel' })
      return
    end
    if not reply.ok or not context.insertable(reply.text) or not matches(s) then
      set_status(reply.ok and (reply.text == nil and 'no_suggestion' or 'stale') or reply.reason)
      invalidate('cancel')
      return
    end
    clear_preview()
    if not show(s, reply.text, reply.word) then
      set_status('stale')
      state.bridge:notify({ op = 'cancel' })
    end
  end)
end

--- Requests a suggestion for the caret now. `explicit` is a user request;
--- on a shown suggestion it asks for another one instead.
function M.revise(mode)
  if not state.config or state.accepting or state.pending then return end
  local reason = blocked_reason(nil, false, true)
  if reason then set_status(reason); return end
  if not ensure_bridge() then set_status('bridge_unavailable'); return end
  if not state.policy then
    local asked = state.generation
    state.bridge:request({ op = 'status' }, function(reply)
      if reply.ok then set_policy(reply) end
      if state.generation == asked and state.policy then M.revise(mode) end
    end)
    return
  end
  reason = policy_reason(true)
  if reason then set_status(reason); return end
  local s = revision.capture()
  if not s then set_status('unsupported_selection'); return end
  local cycle = state.revision_cycle
  mode = mode or (cycle and matches(cycle.snapshot) and cycle.next_mode) or 'grammar'
  if mode ~= 'grammar' and mode ~= 'refine' and mode ~= 'synonyms' then set_status('invalid_request'); return end
  invalidate(nil)
  local generation = state.generation
  state.pending = { generation = generation, snapshot = s }
  set_status('revising')
  state.bridge:request({ op = 'revise', mode = mode, before = s.before, selected = s.selected, after = s.after,
    selection = s.selection, language = state.config.language }, function(reply)
    if state.generation ~= generation then return end
    state.pending = nil
    local current = matches(s)
    if not reply.ok or not current then
      invalidate('cancel')
      state.bridge:notify({ op = 'cancel' })
      set_status(reply.ok and 'stale' or reply.reason)
      if current and reply.reason == 'revision_unavailable' then
        api.nvim_echo({ { 'Badi revisions unavailable — run badi revise' } }, true, {})
      end
      return
    end
    if reply.text == nil and reply.reason ~= 'no_change' then
      invalidate('cancel'); set_status('invalid_response')
      return
    end
    if reply.next_mode ~= 'grammar' and reply.next_mode ~= 'refine' and reply.next_mode ~= 'synonyms' then
      state.bridge:notify({ op = 'cancel' }); set_status('invalid_request'); return
    end
    state.revision_cycle = { snapshot = s, next_mode = reply.next_mode }
    if reply.text == nil then
      set_status('no_change')
      return
    end
    if reply.mode ~= mode or not show_revision(s, reply) then
      invalidate('cancel'); set_status('preview_unavailable')
    end
  end)
end

function M.request(explicit)
  explicit = explicit == true
  if not state.config then return end
  if explicit and api.nvim_get_mode().mode == 'v' then return M.revise() end
  local visible = state.visible
  if explicit and visible and visible.text and not state.accepting and not state.pending and state.bridge
      and matches(visible.snapshot) then
    -- A shown correction has no alternatives; asking again keeps it.
    if visible.replace_before ~= nil then return end
    return alternative()
  end
  local reason = blocked_reason()
  if reason then
    set_status(reason)
    if explicit then api.nvim_echo({ { 'Badi: ' .. reason:gsub('_', ' ') } }, true, {}) end
    return
  end
  if not explicit and not state.config.automatic then return end
  if not ensure_bridge() then
    set_status('bridge_unavailable')
    if explicit then api.nvim_echo({ { 'Badi: the Node bridge could not start; run badi doctor' } }, true, {}) end
    return
  end
  -- Policy before acquisition: until the bridge has reported the policy,
  -- ask it (a content-free status), then decide as if just requested.
  if not state.policy then
    local asked = state.generation
    state.bridge:request({ op = 'status' }, function(reply)
      if reply.ok then set_policy(reply) end
      if state.generation ~= asked then return end
      if state.policy then return M.request(explicit) end
      local why = reply.ok and 'model_offline' or reply.reason
      set_status(why)
      if explicit then api.nvim_echo({ { 'Badi: ' .. tostring(why):gsub('_', ' ') } }, true, {}) end
    end)
    return
  end
  reason = policy_reason(explicit)
  if reason then
    set_status(reason)
    if explicit then api.nvim_echo({ { 'Badi: ' .. reason:gsub('_', ' ') } }, true, {}) end
    return
  end
  local s = snapshot()
  local before, after = context.extract(s.buf, s.row, s.col)
  if not before:find('%S') then return end
  invalidate(nil)
  local generation = state.generation
  state.pending = { generation = generation, snapshot = s }
  set_status('thinking')
  state.bridge:request({
    op = 'suggest', before = before, after = after, explicit = explicit, language = state.config.language,
  }, function(reply)
    if state.generation ~= generation then return end
    state.pending = nil
    if not reply.ok then
      set_status(reply.reason)
      if explicit then api.nvim_echo({ { 'Badi: ' .. tostring(reply.reason):gsub('_', ' ') } }, true, {}) end
      return
    end
    if not context.insertable(reply.text) then
      set_status('no_suggestion')
      if reply.text ~= nil then state.bridge:notify({ op = 'cancel' }) end
      return
    end
    if not matches(s) or not show(s, reply.text, reply.word, reply.replace_before) then
      set_status('stale')
      state.bridge:notify({ op = 'cancel' })
    end
  end)
end

--- Inserts `text` at the snapshot's caret (or replaces the exact suffix
--- `replace_before` ending there) as its own undo step and reports whether
--- the buffer now holds exactly that.
local function apply(s, text, replace_before)
  local start_col = s.col
  if replace_before ~= nil then
    start_col = context.replacement_start(s.line, s.col, replace_before, text)
    if start_col == nil then return false end
  end
  -- Close the undo block of the typing before, so one undo removes only this.
  vim.o.undolevels = vim.o.undolevels
  if not pcall(api.nvim_buf_set_text, s.buf, s.row, start_col, s.row, s.col, { text }) then return false end
  pcall(api.nvim_win_set_cursor, s.win, { s.row + 1, start_col + #text })
  -- And keep typing after it out of this step.
  vim.o.undolevels = vim.o.undolevels
  local line = api.nvim_buf_get_lines(s.buf, s.row, s.row + 1, true)[1]
  local cursor = api.nvim_win_get_cursor(s.win)
  return line == s.line:sub(1, start_col) .. text .. s.line:sub(s.col + 1)
    and cursor[1] == s.row + 1 and cursor[2] == start_col + #text
end

accept = function(acceptance)
  local visible = state.visible
  if not visible or not visible.text or state.accepting or state.pending or not state.bridge then return end
  local s = visible.snapshot
  local generation = state.generation
  state.accepting = true
  local is_revision = s.kind == 'revision'
  state.bridge:request({ op = is_revision and 'accept_revision' or 'accept', acceptance = acceptance }, function(reply)
    if state.generation ~= generation then
      -- The editor moved on while the grant was prepared; it is not used.
      if reply.ok then state.bridge:notify({ op = 'result', status = 'stale' }) end
      return
    end
    state.accepting = false
    if not reply.ok then
      set_status(reply.reason)
      invalidate(nil)
      return
    end
    local text = reply.text
    local replace_before = visible.replace_before
    local expected
    if is_revision then
      expected = text == visible.text and reply.replace_before == nil
        and vim.deep_equal(reply.replace_selection, s.selection)
        and api.nvim_win_is_valid(visible.preview_win) and api.nvim_win_is_valid(s.win)
        and api.nvim_win_get_width(s.win) == visible.owner_width and api.nvim_win_get_height(s.win) == visible.owner_height
        and api.nvim_win_get_width(visible.preview_win) == visible.preview_width
        and api.nvim_win_get_height(visible.preview_win) == visible.preview_height
    elseif replace_before ~= nil then
      -- Either acceptance takes the whole correction, and only for the exact
      -- suffix shown; any other grant replaces nothing.
      expected = text == visible.text and reply.replace_before == replace_before
        and context.replacement_start(s.line, s.col, replace_before, text) ~= nil
    else
      expected = reply.replace_before == nil and (acceptance == 'all' and text == visible.text
        or (acceptance == 'word' and context.insertable(text) and visible.text:sub(1, #text) == text))
    end
    local policy_allowed = state.policy == nil or policy_reason(true) == nil
    local safe = is_revision and revision.safe_text(text) or context.insertable(text)
    if not expected or not policy_allowed or not safe or not matches(s) then
      state.bridge:notify({ op = 'result', status = 'stale' })
      invalidate(nil)
      set_status('stale')
      return
    end
    clear_preview()
    state.generation = state.generation + 1
    state.revision_cycle = nil
    local applied = is_revision and revision.apply(s, text) or (not is_revision and apply(s, text, replace_before))
    state.bridge:notify({ op = 'result', status = applied and 'applied' or 'failed' })
    set_status(applied and 'applied' or 'failed')
  end)
end

local function on_text_changed()
  local s = snapshot()
  local through = context.typed_through(state.shown,
    { buf = s.buf, row = s.row, line_before = s.line:sub(1, s.col) })
  invalidate('cancel')
  if not state.config.automatic or blocked_reason(s.buf) then return end
  state.timer = vim.uv.new_timer()
  state.scheduled = { buf = s.buf, tick = s.tick, row = s.row, col = s.col }
  local generation = state.generation
  state.timer:start(through and TYPE_THROUGH_DELAY_MS or state.config.delay_ms, 0, vim.schedule_wrap(function()
    if state.generation ~= generation then return end
    stop_timer()
    M.request(false)
  end))
end

local function on_cursor_moved()
  local active = state.visible and state.visible.snapshot or state.pending and state.pending.snapshot
    or state.revision_cycle and state.revision_cycle.snapshot
  if active then
    if not matches(active) then invalidate('cancel') end
    return
  end
  local scheduled = state.scheduled
  if scheduled then
    local s = snapshot()
    -- Moving the caret without typing cancels the armed request.
    if s.buf ~= scheduled.buf or (s.tick == scheduled.tick and (s.row ~= scheduled.row or s.col ~= scheduled.col)) then
      stop_timer()
    end
  end
end

local function dismiss()
  if state.visible and not state.accepting then
    invalidate('dismiss')
  else
    invalidate('cancel')
  end
  state.shown = nil
end

function M.setup(options)
  state.config = vim.tbl_deep_extend('force', vim.deepcopy(defaults), options or {})
  if state.bridge then
    invalidate('cancel')
    state.bridge:stop()
    state.bridge = nil
  end
  api.nvim_set_hl(0, 'BadiSuggestion', { link = 'Comment', default = true })
  api.nvim_set_hl(0, 'BadiRevisionRemoved', { link = 'DiffDelete', default = true })
  api.nvim_set_hl(0, 'BadiRevisionAdded', { link = 'DiffAdd', default = true })
  -- The word Tab takes: the comment colour, bold, unless the user set it.
  local ok, comment = pcall(api.nvim_get_hl, 0, { name = 'Comment', link = false })
  api.nvim_set_hl(0, 'BadiSuggestionWord', { fg = ok and comment.fg or nil, bold = true, default = true })
  local group = api.nvim_create_augroup('Badi', { clear = true })
  local autocmd = function(events, callback) api.nvim_create_autocmd(events, { group = group, callback = callback }) end
  autocmd('InsertEnter', function()
    -- Start the bridge before the first pause in a prose buffer, never elsewhere.
    if state.config.automatic and blocked_reason(nil, true) == nil then ensure_bridge() end
  end)
  autocmd('TextChangedI', on_text_changed)
  autocmd('CursorMovedI', on_cursor_moved)
  autocmd('CursorMoved', on_cursor_moved)
  autocmd({ 'TextChanged', 'ModeChanged' }, function()
    local active = state.pending and state.pending.snapshot or state.visible and state.visible.snapshot
      or state.revision_cycle and state.revision_cycle.snapshot
    if active and active.kind == 'revision' and not matches(active) then invalidate('cancel') end
  end)
  autocmd({ 'WinResized', 'WinScrolled' }, function()
    -- Window geometry is part of the marked preview's visibility proof.
    if state.visible and state.visible.snapshot.kind == 'revision' then invalidate('cancel') end
  end)
  autocmd('WinClosed', function()
    local visible = state.visible
    if visible and visible.preview_win and not api.nvim_win_is_valid(visible.preview_win) then invalidate('cancel') end
  end)
  autocmd('InsertLeave', dismiss)
  autocmd({ 'TextChangedP', 'CompleteChanged', 'BufLeave', 'WinLeave' }, function() invalidate('cancel') end)
  autocmd('FocusLost', function()
    state.focused = false
    invalidate('cancel')
    hold_terminal(false)
  end)
  autocmd('FocusGained', function()
    state.focused = true
    hold_terminal(true)
  end)
  -- Ctrl+Z hands the terminal back to the shell; `fg` returns it.
  local resumes = false
  autocmd('VimSuspend', function()
    resumes = state.holds_terminal
    hold_terminal(false)
  end)
  autocmd('VimResume', function() if resumes then hold_terminal(true) end end)
  autocmd('VimLeavePre', function()
    invalidate('cancel')
    if state.bridge then state.bridge:stop() end
  end)
  local request = state.config.keys.request
  if request and request ~= '' and vim.fn.maparg(request, 'i') == '' then
    vim.keymap.set('i', request, function() M.request(true) end, { silent = true, desc = 'Badi: request a suggestion' })
  end
  if request and request ~= '' and vim.fn.maparg(request, 'x') == '' then
    vim.keymap.set('x', request, function() M.request(true) end, { silent = true, desc = 'Badi: revise the selection' })
  end
  -- Only while an explicit selection request exists: an unrelated key
  -- retires it even when that key leaves the caret in place. No key text
  -- is stored or sent to the broker.
  vim.on_key(function(_, typed)
    if not typed or typed == '' then return end
    local active = state.pending and state.pending.snapshot or state.visible and state.visible.snapshot
      or state.revision_cycle and state.revision_cycle.snapshot
    if not active or active.kind ~= 'revision' then return end
    local function is_key(lhs) return lhs and typed == api.nvim_replace_termcodes(lhs, true, false, true) end
    if is_key(request) or (state.visible and is_key(state.config.keys.accept_word)) then return end
    local generation = state.generation
    vim.schedule(function() if state.generation == generation then invalidate('dismiss') end end)
  end, namespace)
end

function M.is_setup()
  return state.config ~= nil
end

--- Restarts the bridge with fresh policy; nothing from the old one is kept.
function M.reconnect()
  invalidate('cancel')
  state.shown = nil
  if state.bridge then state.bridge:stop() end
  state.bridge = nil
  return ensure_bridge()
end

--- Content-free status: the last reason and whether a preview is showing.
function M.status()
  return {
    status = state.status,
    bridge = state.bridge ~= nil and state.bridge:running(),
    pending = state.pending ~= nil,
    visible = state.visible ~= nil,
    accepting = state.accepting,
    holds_terminal = state.holds_terminal,
    policy = not state.policy and 'unknown' or not state.policy.allowed and 'refused'
      or state.policy.automatic and 'automatic' or 'manual',
    blocked = not state.config and 'not_set_up' or blocked_reason(nil, false, api.nvim_get_mode().mode == 'v'),
  }
end

--- For tests only: the preview text currently shown, if any.
function M._preview()
  return state.visible and (state.visible.display or state.visible.text or state.visible.list_text) or nil
end

--- The shown preview's virtual text chunks, for tests.
function M._preview_chunks()
  local visible = state.visible
  if not visible then return nil end
  if visible.chunks then return visible.chunks end
  local ok, mark = pcall(api.nvim_buf_get_extmark_by_id, visible.snapshot.buf, namespace, visible.mark,
    { details = true })
  return ok and mark[3] and mark[3].virt_text or nil
end

return M
