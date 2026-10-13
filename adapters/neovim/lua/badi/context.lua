-- Buffer eligibility and bounded context, without side effects. Offsets are
-- Neovim's: rows are 0-based, columns are byte offsets into UTF-8 lines.
local M = {}

-- The broker reads at most 512 Unicode scalars before the caret and only the
-- rest of the caret's line after it. Read enough bytes for that and no more:
-- a scalar is at most four bytes.
M.BEFORE_BYTES = 2048
M.AFTER_BYTES = 512
M.MAX_LINES = 64

-- Buffers whose text is not a person's prose, or that cannot be edited.
local WRITABLE_BUFTYPES = { [''] = true, acwrite = true }
-- Badi cannot tell a secret from its content. Password-store and encrypted
-- files are refused by where they live and how they are named.
local SENSITIVE_PATTERNS = { '^/dev/shm/', '%.gpg$', '%.asc$', '%.pgp$', '%.age$' }

local function continuation(byte)
  return byte ~= nil and byte >= 0x80 and byte < 0xC0
end

--- The last `limit` bytes of `text`, starting on a UTF-8 boundary.
function M.tail(text, limit)
  if #text <= limit then return text end
  local start = #text - limit + 1
  while start <= #text and continuation(text:byte(start)) do start = start + 1 end
  return text:sub(start)
end

--- The first `limit` bytes of `text`, ending on a UTF-8 boundary.
function M.head(text, limit)
  if #text <= limit then return text end
  local stop = limit
  while stop > 0 and continuation(text:byte(stop + 1)) do stop = stop - 1 end
  return text:sub(1, stop)
end

--- Why `buf` in `win` cannot be read now, or nil when it can.
--- `facts` carries editor state that is not a buffer option, so this stays
--- testable: { mode, pumvisible, executing, focused }.
function M.blocked_reason(buf, config, facts)
  if vim.g.badi_disable == true or vim.b[buf].badi_disable == true then return 'disabled' end
  if facts.mode ~= 'i' then return 'not_insert_mode' end
  if facts.pumvisible then return 'completion_menu' end
  if facts.executing then return 'macro_running' end
  if not facts.focused then return 'editor_not_focused' end
  local bo = vim.bo[buf]
  if not WRITABLE_BUFTYPES[bo.buftype] then return 'special_buffer' end
  if not bo.modifiable or bo.readonly then return 'read_only' end
  if bo.binary then return 'binary_buffer' end
  if vim.o.paste then return 'paste_mode' end
  local name = vim.api.nvim_buf_get_name(buf)
  for _, pattern in ipairs(SENSITIVE_PATTERNS) do
    if name:find(pattern) then return 'sensitive_buffer' end
  end
  if not vim.tbl_contains(config.filetypes, bo.filetype) then return 'filetype_not_prose' end
  return nil
end

--- The text before and after a caret at (`row`, `col`), bounded as above.
--- Lines are joined with "\n" exactly as the buffer stores them.
function M.extract(buf, row, col)
  local line = vim.api.nvim_buf_get_lines(buf, row, row + 1, true)[1]
  local parts = { line:sub(1, col) }
  local size = col
  local first = row
  while first > 0 and size < M.BEFORE_BYTES and row - first < M.MAX_LINES do
    first = first - 1
    local previous = vim.api.nvim_buf_get_lines(buf, first, first + 1, true)[1]
    table.insert(parts, 1, previous)
    size = size + #previous + 1
  end
  local before = M.tail(table.concat(parts, '\n'), M.BEFORE_BYTES)
  local after = M.head(line:sub(col + 1), M.AFTER_BYTES)
  return before, after
end

--- Whether the text typed since a suggestion was shown is a proper prefix of
--- it, so the broker can carry the rest without a model call (ADR 0004).
--- `shown` = { buf, row, line_before, text }; `now` = { buf, row, line_before }.
function M.typed_through(shown, now)
  if not shown or shown.buf ~= now.buf or shown.row ~= now.row then return false end
  local base = shown.line_before
  if #now.line_before <= #base or now.line_before:sub(1, #base) ~= base then return false end
  local typed = now.line_before:sub(#base + 1)
  return #typed < #shown.text and shown.text:sub(1, #typed) == typed
end

--- A continuation Neovim may insert: one line of text with no control
--- characters. The broker and the shared client already enforce the rest.
function M.insertable(text)
  return type(text) == 'string' and text ~= '' and #text <= 256 and not text:find('%c')
end

return M
