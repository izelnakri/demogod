#!/usr/bin/env node
// `npx demogod …`: the native binary, with this process's arguments, streams and exit code.
import { spawn } from 'node:child_process';
import { binaryPath } from '../binary.js';

let binary;
try {
  binary = binaryPath();
} catch (error) {
  console.error(`error: ${error.message}`);
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', (error) => {
  console.error(`error: could not run ${binary}: ${error.message}`);
  process.exit(1);
});
// Ended by a signal, this process ends the same way, so its parent sees why.
child.on('exit', (code, signal) => {
  if (!signal) process.exit(code ?? 1);
  process.removeAllListeners(signal);
  process.kill(process.pid, signal);
});
