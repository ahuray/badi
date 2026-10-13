// Drives a headless Neovim with this plugin through its RPC socket, using
// only `nvim --server` so no client library is needed.
import { spawn, execFile, spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile, lstat } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { promisify } from 'node:util';
import { once } from 'node:events';

export const ROOT = resolve(import.meta.dirname, '../../..');
export const PLUGIN = resolve(import.meta.dirname, '..');
const run = promisify(execFile);
export const delay = ms => new Promise(done => setTimeout(done, ms));

// Inline virtual text needs Neovim 0.10; report a skip where it is missing,
// unless BADI_REQUIRE_NEOVIM=1 makes that a failure (the Neovim CI job).
export function neovimSkipReason() {
  const probe = spawnSync('nvim', ['--headless', '--clean', '-es', '+lua io.write(vim.fn.has("nvim-0.10"))', '+qa!'],
    { encoding: 'utf8', timeout: 10000 });
  const reason = probe.error ? 'Neovim is not installed'
    : probe.stdout.trim() === '1' ? false : 'Neovim 0.10 or newer is required';
  if (reason && process.env.BADI_REQUIRE_NEOVIM === '1') throw new Error(reason);
  return reason;
}

const INIT = String.raw`
vim.opt.runtimepath:prepend(vim.env.BADI_PLUGIN)
vim.o.shada = ''
vim.o.swapfile = false
vim.o.columns = 100
vim.o.lines = 20
vim.cmd.runtime('plugin/badi.lua')
function _G.badi_test_state()
  local badi = require('badi')
  local cursor = vim.api.nvim_win_get_cursor(0)
  local screen = {}
  local position = vim.fn.screenpos(0, cursor[1], 1)
  if position.row > 0 then
    for column = 1, vim.o.columns do screen[#screen + 1] = vim.fn.screenstring(position.row, column) end
  end
  local rows = {}
  for row = 1, vim.o.lines do
    local cells = {}
    for column = 1, vim.o.columns do cells[#cells + 1] = vim.fn.screenstring(row, column) end
    rows[#rows + 1] = table.concat(cells):gsub('%s+$', '')
  end
  return vim.json.encode({
    rows = rows,
    mode = vim.api.nvim_get_mode().mode,
    lines = vim.api.nvim_buf_get_lines(0, 0, -1, true),
    cursor = cursor,
    preview = badi._preview() or vim.NIL,
    status = badi.status(),
    screen = table.concat(screen):gsub('%s+$', ''),
    messages = vim.fn.execute('messages'),
  })
end
`;

export async function startNeovim({ env = {}, config = {} } = {}) {
  const root = await mkdtemp('/tmp/badi-neovim-test-');
  const socket = join(root, 'nvim.sock');
  const init = join(root, 'init.lua');
  await writeFile(init, `vim.g.badi = vim.json.decode(vim.env.BADI_TEST_CONFIG)\n${INIT}`);
  const environment = { ...process.env, ...env, BADI_PLUGIN: PLUGIN, BADI_TEST_CONFIG: JSON.stringify(config),
    XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'), XDG_STATE_HOME: join(root, 'state') };
  const nvim = spawn('nvim', ['--headless', '--clean', '-u', init, '--listen', socket],
    { env: environment, stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = '';
  nvim.stderr.on('data', data => { stderr += data; });
  const deadline = performance.now() + 5000;
  while (await lstat(socket).then(() => false, () => true)) {
    if (performance.now() > deadline || nvim.exitCode !== null) throw new Error(`Neovim did not start: ${stderr}`);
    await delay(20);
  }
  const server = args => run('nvim', ['--server', socket, ...args], { env: environment, encoding: 'utf8', timeout: 5000 });
  const editor = {
    root,
    environment,
    send: keys => server(['--remote-send', keys]),
    expr: async expression => (await server(['--remote-expr', expression])).stdout,
    command: command => editor.expr(`execute(${JSON.stringify(command)})`),
    state: async () => JSON.parse(await editor.expr('v:lua.badi_test_state()')),
    async until(predicate, label) {
      const end = performance.now() + 4000;
      let current;
      while (performance.now() < end) {
        current = await editor.state();
        if (predicate(current)) return current;
        await delay(15);
      }
      throw new Error(`Timed out waiting for ${label}: ${JSON.stringify(current)}`);
    },
    async close() {
      if (nvim.exitCode === null) {
        const exited = once(nvim, 'exit');
        await editor.send('<C-\\><C-N>:qa!<CR>').catch(() => undefined);
        const timer = setTimeout(() => nvim.kill('SIGKILL'), 3000);
        await exited;
        clearTimeout(timer);
      }
      await rm(root, { recursive: true, force: true });
      return stderr;
    },
  };
  return editor;
}
