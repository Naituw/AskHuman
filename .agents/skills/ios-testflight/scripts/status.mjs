#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

if (['--help', '-h'].includes(process.argv[2])) {
  console.log('Usage: node status.mjs [app-store-connect.json]\nRead-only: query Apple through the signed Keychain helper; no credential contents are returned.');
} else {
  const driver = fileURLToPath(new URL('./helper.py', import.meta.url));
  const args = [driver, 'status'];
  if (process.argv[2]) args.push('--config', process.argv[2]);
  const result = spawnSync('python3', args, { stdio: 'inherit' });
  if (result.error) console.error(result.error.message);
  process.exitCode = result.status ?? 1;
}
