import { build } from 'esbuild';
import { mkdir, copyFile } from 'node:fs/promises';
import { join } from 'node:path';
const root = import.meta.dirname;
const output = join(root, 'dist');
await mkdir(output, { recursive: true });
await build({ entryPoints: [join(root, 'main.mjs')], outfile: join(output, 'main.js'),
  bundle: true, platform: 'node', format: 'cjs', target: 'es2022',
  external: ['obsidian', '@codemirror/view', '@codemirror/state', '@codemirror/commands', '@codemirror/autocomplete'],
  logLevel: 'warning' });
for (const name of ['manifest.json', 'styles.css']) await copyFile(join(root, name), join(output, name));
