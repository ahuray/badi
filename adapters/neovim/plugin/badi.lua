-- Badi for Neovim. Inline virtual text needs Neovim 0.10 or newer.
if vim.g.loaded_badi or vim.fn.has('nvim-0.10') == 0 then return end
vim.g.loaded_badi = true

vim.api.nvim_create_user_command('BadiRequest', function() require('badi').request(true) end,
  { desc = 'Badi: ask for a suggestion at the caret' })
vim.api.nvim_create_user_command('BadiReconnect', function() require('badi').reconnect() end,
  { desc = 'Badi: restart the broker bridge with fresh policy' })
vim.api.nvim_create_user_command('BadiStatus', function()
  local status = require('badi').status()
  vim.api.nvim_echo({ { string.format('Badi: %s · bridge %s · %s', status.status,
    status.bridge and 'running' or 'stopped', status.blocked and status.blocked:gsub('_', ' ') or 'eligible') } },
    false, {})
end, { desc = 'Badi: content-free status' })

-- An explicit require('badi').setup() in init.lua wins; otherwise use the
-- defaults merged with vim.g.badi. Set vim.g.badi_auto_setup = false to skip.
if vim.g.badi_auto_setup ~= false and not require('badi').is_setup() then
  require('badi').setup(vim.g.badi or {})
end
