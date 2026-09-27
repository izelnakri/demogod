// demogod from JavaScript: records a tape with the native binary, and lets `Do <name>` in the tape
// call JavaScript functions.
//
// The binary does all the work. This file starts it with `--json`, reads its progress one JSON
// line at a time, and answers when it asks for an action to be run.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { binaryPath } from './binary.js';

export { binaryPath };

/** A failed recording, with where in the tape it failed when it was the tape's doing. */
export class DemogodError extends Error {
  /**
   * @param {string} message
   * @param {{ location?: { file: string, line: number }, help?: string, text?: string }} [details]
   */
  constructor(message, { location, help, text } = {}) {
    super(message);
    this.name = 'DemogodError';
    /** The tape line it failed on. */
    this.location = location ?? undefined;
    /** How to fix it, when that is clear. */
    this.help = help ?? undefined;
    /** The whole report, the way the command line prints it: message, line, excerpt and help. */
    this.text = text ?? message;
  }
}

/**
 * A tape written in JavaScript, as a template literal. Indentation common to every line is
 * removed, and nothing is escaped, so it reads exactly as a `.tape` file would.
 *
 * ```js
 * await record(tape`
 *   Output demo.gif
 *   Type "npm test"
 *   Enter
 *   Wait /passing/
 * `);
 * ```
 *
 * Relative paths in it are relative to `options.cwd`, or the current directory.
 *
 * @param {TemplateStringsArray} strings
 * @param {...unknown} values
 * @returns {{ source: string }}
 */
export function tape(strings, ...values) {
  const text = String.raw({ raw: strings.raw }, ...values);
  const lines = text.replace(/^\s*\n/, '').replace(/\n\s*$/, '').split('\n');
  const indent = Math.min(...lines.filter((line) => line.trim()).map((line) => line.match(/^\s*/)[0].length));

  return { source: lines.map((line) => line.slice(Number.isFinite(indent) ? indent : 0)).join('\n') + '\n' };
}

/**
 * Records a tape and saves it to its `Output` files, or to `options.output` instead.
 *
 * ```js
 * import { record } from 'demogod';
 *
 * const saved = await record('demo.tape', {
 *   actions: { break: () => writeFile('src/cart.ts', broken) },
 *   onEvent: (event) => event.type === 'scene' && console.log(event.title),
 * });
 * console.log(saved[0].path, saved[0].bytes);
 * ```
 *
 * @param {string | { source: string }} tape a `.tape` file, or {@link tape} source
 * @param {import('./index.d.ts').RecordOptions} [options]
 * @returns {Promise<import('./index.d.ts').Saved[]>}
 */
export async function record(tape, options = {}) {
  const outputs = [options.output ?? []].flat();
  const events = await run([...tapeArgument(tape), ...outputs.flatMap((path) => ['--output', path])], tape, options);

  return events.filter((event) => event.type === 'saved').map(({ type: _, ...saved }) => saved);
}

/**
 * Reads a tape and checks it can be recorded — every `Do` has an action, every `Require`d
 * program is installed, every output format can be written — without recording anything.
 *
 * @param {string | { source: string }} tape
 * @param {Pick<import('./index.d.ts').RecordOptions, 'actions' | 'cwd' | 'signal'>} [options]
 * @returns {Promise<{ scenes: number, steps: number }>}
 */
export async function check(tape, options = {}) {
  const events = await run(['check', ...tapeArgument(tape)], tape, { ...options, onEvent: undefined });
  const checked = events.find((event) => event.type === 'checked');
  if (!checked) throw new DemogodError('demogod checked the tape but reported nothing');

  return { scenes: checked.scenes, steps: checked.steps };
}

/**
 * The names of the built-in themes, for `Set Theme`.
 *
 * @returns {Promise<string[]>}
 */
export async function themes() {
  return new Promise((resolve, reject) => {
    const child = spawn(binaryPath(), ['themes'], { stdio: ['ignore', 'pipe', 'inherit'] });
    let output = '';
    child.stdout.on('data', (chunk) => (output += chunk));
    child.on('error', reject);
    child.on('close', (code) =>
      code === 0 ? resolve(output.split('\n').filter(Boolean)) : reject(new DemogodError(`demogod exited with ${code}`)),
    );
  });
}

function tapeArgument(tape) {
  if (typeof tape === 'string') return [tape];
  if (tape && typeof tape.source === 'string') return ['-'];
  throw new TypeError('a tape is a path to a .tape file, or tape`…` source');
}

/** Stops the binary and everything it started: its process group, or on Windows its tree. */
function stop(child) {
  try {
    if (process.platform === 'win32') spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    else process.kill(-child.pid, 'SIGTERM');
  } catch {
    child.kill();
  }
}

/** Runs the binary with `--json`, answering its action requests, and returns every event. */
function run(args, tape, { actions = {}, onEvent, cwd, signal } = {}) {
  return new Promise((resolve, reject) => {
    const names = Object.keys(actions);
    // Its own process group on Unix, so stopping it stops the browser it started too.
    const child = spawn(binaryPath(), [...args, '--json', ...names.flatMap((name) => ['--action', name])], {
      cwd,
      stdio: ['pipe', 'pipe', 'pipe'],
      detached: process.platform !== 'win32',
    });
    const events = [];
    let failure;
    let stderr = '';

    if (typeof tape === 'object') child.stdin.write(JSON.stringify({ tape: tape.source }) + '\n');
    if (names.length === 0) child.stdin.end();

    const abort = () => stop(child);
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();

    // A child that dies while an answer is on its way closes stdin; that is reported on close.
    child.stdin.on('error', () => {});

    // Lines are handled one at a time, in order: an action has finished before the next line is.
    // Anything that throws — an `onEvent` — ends the recording with that error.
    let queue = Promise.resolve();
    createInterface({ input: child.stdout }).on('line', (line) => {
      queue = queue.then(() => handle(line)).catch((error) => {
        failure ??= error;
        stop(child);
      });
    });

    async function handle(line) {
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        return;
      }
      const { event: type, ...fields } = message;
      if (type === 'action-request') {
        let answer = { ok: true };
        try {
          await actions[fields.name]();
        } catch (error) {
          answer = { error: error instanceof Error ? error.message : String(error) };
        }
        if (!child.stdin.destroyed) child.stdin.write(JSON.stringify(answer) + '\n');
        return;
      }
      if (type === 'error') {
        failure = new DemogodError(fields.message, fields);
        return;
      }
      const event = { type, ...fields };
      events.push(event);
      onEvent?.(event);
    }

    child.stderr.on('data', (chunk) => (stderr += chunk));
    child.on('error', (error) => reject(new DemogodError(`could not run demogod: ${error.message}`)));
    child.on('close', async (code) => {
      signal?.removeEventListener('abort', abort);
      await queue;
      if (signal?.aborted) return reject(signal.reason ?? new DemogodError('aborted'));
      if (code === 0 && !failure) return resolve(events);
      reject(failure ?? new DemogodError(stderr.trim().replace(/^error: /, '') || `demogod exited with ${code}`));
    });
  });
}
