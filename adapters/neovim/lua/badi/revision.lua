-- Exact characterwise Visual selections. Buffer eligibility and broker
-- policy are checked by the caller before any of these helpers read text.
local context = require('badi.context')
local api = vim.api
local M = {}

function M.safe_text(text, limit)
  return type(text) == 'string' and text ~= '' and #text <= (limit or 1100) * 4
    and vim.fn.strchars(text) <= (limit or 1100) and not text:find('%c') and not text:find('�', 1, true)
end

function M.capture()
  if api.nvim_get_mode().mode ~= 'v' or vim.fn.exists('*getregion') == 0
      or vim.fn.exists('*getregionpos') == 0 then return nil end
  local win, buf = api.nvim_get_current_win(), api.nvim_get_current_buf()
  local anchor, head = vim.fn.getpos('v'), vim.fn.getpos('.')
  if anchor[2] ~= head[2] or anchor[4] ~= 0 or head[4] ~= 0 then return nil end
  local options = { type = 'v', exclusive = vim.o.selection == 'exclusive' }
  local positions = vim.fn.getregionpos(anchor, head, options)
  local text = vim.fn.getregion(anchor, head, options)
  if #positions ~= 1 or #text ~= 1 or not M.safe_text(text[1], 600) or not text[1]:find('%S') then return nil end
  local start, finish = positions[1][1], positions[1][2]
  if start[4] ~= 0 or finish[4] ~= 0 or start[3] <= 0 then return nil end
  local row, first = start[2] - 1, start[3] - 1
  local line = api.nvim_buf_get_lines(buf, row, row + 1, true)[1]
  local last = first + #text[1]
  -- Virtual space and partial screen cells never become invented buffer text.
  if last > #line or line:sub(first + 1, last) ~= text[1] then return nil end
  local offset = vim.fn.strchars(line:sub(1, first))
  local ending = offset + vim.fn.strchars(text[1])
  local backwards = anchor[3] > head[3]
  local before = context.extract(buf, row, first)
  return { kind = 'revision', win = win, buf = buf, tick = api.nvim_buf_get_changedtick(buf),
    row = row, col = head[3] - 1, first = first, last = last, line = line,
    anchor = anchor, head = head, selection_option = vim.o.selection, selected = text[1],
    before = before, after = context.head(line:sub(last + 1), context.AFTER_BYTES),
    selection = { anchor = backwards and ending or offset, head = backwards and offset or ending,
      unit = 'unicode_scalar_values' } }
end

function M.matches(s)
  return api.nvim_get_mode().mode == 'v' and vim.o.selection == s.selection_option
    and vim.deep_equal(vim.fn.getpos('v'), s.anchor) and vim.deep_equal(vim.fn.getpos('.'), s.head)
end

-- Both reconstructions must equal the actual selection and offered text;
-- a malformed display never authorizes a replacement the user did not see.
function M.preview(s, reply, width, max_lines)
  if not M.safe_text(reply.text) or type(reply.diff) ~= 'table' then return nil end
  local original, revised, chunks = {}, {}, { { 'Badi ' .. reply.mode .. ': ', 'BadiSuggestion' } }
  local groups = { same = 'BadiSuggestion', removed = 'BadiRevisionRemoved', added = 'BadiRevisionAdded' }
  for _, part in ipairs(reply.diff) do
    if type(part) ~= 'table' or not groups[part.kind] or not M.safe_text(part.text) then return nil end
    if part.kind ~= 'added' then original[#original + 1] = part.text end
    if part.kind ~= 'removed' then revised[#revised + 1] = part.text end
    local text = part.kind == 'removed' and ('[-' .. part.text .. ']')
      or part.kind == 'added' and ('[+' .. part.text .. ']') or part.text
    chunks[#chunks + 1] = { text, groups[part.kind] }
  end
  if table.concat(original) ~= s.selected or table.concat(revised) ~= reply.text then return nil end
  chunks[#chunks + 1] = { ' · Tab apply · Esc dismiss', 'BadiSuggestion' }
  local lines, line, used, display = {}, {}, 0, {}
  for _, chunk in ipairs(chunks) do
    display[#display + 1] = chunk[1]
    for index = 0, vim.fn.strchars(chunk[1]) - 1 do
      local character = vim.fn.strcharpart(chunk[1], index, 1)
      local cells = vim.fn.strdisplaywidth(character)
      if used + cells > width then lines[#lines + 1], line, used = line, {}, 0 end
      local last = line[#line]
      if last and last[2] == chunk[2] then last[1] = last[1] .. character
      else line[#line + 1] = { character, chunk[2] } end
      used = used + cells
    end
  end
  if #line > 0 then lines[#lines + 1] = line end
  if #lines > max_lines then return nil end
  return { lines = lines, chunks = chunks, display = table.concat(display) }
end

function M.apply(s, text)
  if not M.safe_text(text) then return false end
  local undo = vim.bo[s.buf].undolevels
  vim.bo[s.buf].undolevels = undo
  if not pcall(api.nvim_buf_set_text, s.buf, s.row, s.first, s.row, s.last, { text }) then return false end
  -- Leave Visual mode synchronously through the editor command; nothing
  -- remains queued ahead of later user input, and registers are untouched.
  vim.cmd.normal({ args = { api.nvim_replace_termcodes('<Esc>', true, false, true) }, bang = true })
  pcall(api.nvim_win_set_cursor, s.win, { s.row + 1, s.first + #text })
  vim.bo[s.buf].undolevels = undo
  return api.nvim_buf_get_lines(s.buf, s.row, s.row + 1, true)[1]
    == s.line:sub(1, s.first) .. text .. s.line:sub(s.last + 1)
end

return M
