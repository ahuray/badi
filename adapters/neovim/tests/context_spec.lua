-- Run with: nvim --headless --clean -l adapters/neovim/tests/context_spec.lua
vim.opt.runtimepath:prepend(vim.fn.fnamemodify(debug.getinfo(1, 'S').source:sub(2), ':p:h:h'))
local context = require('badi.context')

-- Byte bounds never split a UTF-8 sequence.
local persian = 'سلام' -- four two-byte letters
assert(context.tail(persian, 3) == 'م', context.tail(persian, 3))
assert(context.tail(persian, 4) == 'ام')
assert(context.head(persian, 3) == 'س')
assert(context.head('Grüße', 3) == 'Gr')
assert(context.head('Grüße', 4) == 'Grü')
assert(context.tail('🙂x', 4) == 'x')
assert(context.head('🙂x', 3) == '')
assert(context.tail('plain', 10) == 'plain' and context.head('plain', 10) == 'plain')

-- Extraction joins lines exactly and bounds both sides.
local buf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(buf, 0, -1, true, { 'first', 'Grüße und سلام ', 'last line' })
local before, after = context.extract(buf, 1, #'Grüße und ')
assert(before == 'first\nGrüße und ', before)
assert(after == 'سلام ', after)
vim.api.nvim_buf_set_lines(buf, 0, -1, true, { string.rep('ä', 3000), 'x' })
before, after = context.extract(buf, 1, 1)
assert(#before <= context.BEFORE_BYTES and before:sub(-2) == '\nx', #before)
assert(vim.str_utf_start(before, 1) == 0, 'starts on a boundary')
assert(after == '')
local lines = {}
for index = 1, 200 do lines[index] = 'l' end
vim.api.nvim_buf_set_lines(buf, 0, -1, true, lines)
before = context.extract(buf, 199, 1)
assert(select(2, before:gsub('\n', '')) == context.MAX_LINES, 'at most MAX_LINES earlier lines')

-- Type-through: only a proper prefix of the shown text on the same line.
local shown = { buf = 1, row = 0, line_before = 'thank you', text = ' for your time' }
assert(context.typed_through(shown, { buf = 1, row = 0, line_before = 'thank you f' }))
assert(context.typed_through(shown, { buf = 1, row = 0, line_before = 'thank you for your' }))
assert(not context.typed_through(shown, { buf = 1, row = 0, line_before = 'thank you for your time' }))
assert(not context.typed_through(shown, { buf = 1, row = 0, line_before = 'thank you x' }))
assert(not context.typed_through(shown, { buf = 2, row = 0, line_before = 'thank you f' }))
assert(not context.typed_through(shown, { buf = 1, row = 1, line_before = 'thank you f' }))
assert(not context.typed_through(nil, { buf = 1, row = 0, line_before = 'x' }))

-- Insertable text is one line without control characters.
assert(context.insertable(' for your time'))
assert(context.insertable('Welt bleibt'))
for _, value in ipairs({ '', 'a\nb', 'a\rb', 'a\tb', 'a\0b', 42, string.rep('a', 257) }) do
  assert(not context.insertable(value), vim.inspect(value))
end
io.write('context helpers ok\n')
