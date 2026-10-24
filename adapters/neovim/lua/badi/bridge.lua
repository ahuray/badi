-- One Node bridge process per Neovim, speaking newline-delimited JSON over
-- its stdio. Job callbacks run on Neovim's main loop.
local Bridge = {}
Bridge.__index = Bridge

local LINE_LIMIT = 65536
local REPLY_TIMEOUT_MS = 4000

function Bridge.new(options)
  return setmetatable({
    command = options.command,
    on_event = options.on_event,
    on_exit = options.on_exit,
    job = nil,
    partial = '',
    next_id = 1,
    pending = {},
  }, Bridge)
end

function Bridge:running()
  return self.job ~= nil
end

function Bridge:start()
  if self.job then return true end
  local ok, job = pcall(vim.fn.jobstart, self.command, {
    on_stdout = function(_, data) self:_receive(data) end,
    -- The bridge never writes typed text to stderr; discard whatever Node may print.
    on_stderr = function() end,
    on_exit = function(id) if self.job == id then self:_closed() end end,
  })
  if not ok or type(job) ~= 'number' or job <= 0 then return false end
  self.job = job
  self.partial = ''
  return true
end

function Bridge:stop()
  local job = self.job
  if not job then return end
  self:_closed()
  pcall(vim.fn.jobstop, job)
end

function Bridge:_closed()
  self.job = nil
  self.partial = ''
  local pending = self.pending
  self.pending = {}
  for _, callback in pairs(pending) do callback({ ok = false, reason = 'model_offline' }) end
  if self.on_exit then self.on_exit() end
end

function Bridge:_write(message)
  if not self.job then return false end
  local ok, encoded = pcall(vim.json.encode, message)
  if not ok then return false end
  return pcall(vim.fn.chansend, self.job, encoded .. '\n') and true or false
end

--- Sends a message without waiting for its reply.
function Bridge:notify(message)
  return self:_write(message)
end

--- Sends a message; `callback(reply)` runs exactly once, on the reply, an
--- exited bridge or a timeout.
function Bridge:request(message, callback)
  local id = self.next_id
  self.next_id = id + 1
  message.id = id
  local done = false
  local function finish(reply)
    if done then return end
    done = true
    self.pending[id] = nil
    callback(reply)
  end
  self.pending[id] = finish
  if not self:_write(message) then
    finish({ ok = false, reason = 'model_offline' })
    return
  end
  vim.defer_fn(function() finish({ ok = false, reason = 'timeout' }) end, REPLY_TIMEOUT_MS)
end

function Bridge:_receive(data)
  -- Each element ends a line except the last, which is a partial line.
  self.partial = self.partial .. data[1]
  for index = 2, #data do
    self:_line(self.partial)
    self.partial = data[index]
  end
  if #self.partial > LINE_LIMIT then self:stop() end
end

function Bridge:_line(line)
  if line == '' then return end
  local ok, message = pcall(vim.json.decode, line, { luanil = { object = true, array = true } })
  if not ok or type(message) ~= 'table' then return end
  if message.event ~= nil then
    if self.on_event then self.on_event(message) end
    return
  end
  local callback = type(message.id) == 'number' and self.pending[message.id]
  if callback then callback(message) end
end

return Bridge
