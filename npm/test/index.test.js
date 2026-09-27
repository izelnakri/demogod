import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { binaryPath, check, DemogodError, record, tape, themes } from '../index.js';

// The binary under test: DEMOGOD_BINARY, or the one `cargo build` leaves in target/.
const executable = process.platform === 'win32' ? 'demogod.exe' : 'demogod';
const built = fileURLToPath(new URL(`../../target/debug/${executable}`, import.meta.url));
process.env.DEMOGOD_BINARY ??= built;
const skip = existsSync(process.env.DEMOGOD_BINARY) ? false : `no binary at ${process.env.DEMOGOD_BINARY}: cargo build first`;
const unixShell = process.platform === 'win32' ? 'records a POSIX shell' : false;

const SMALL = 'Set Width 400\nSet Height 200\nSet FontSize 12\nSet Padding 8\n';

let directory;
before(async () => (directory = await mkdtemp(path.join(tmpdir(), 'demogod-npm-'))));
after(() => rm(directory, { recursive: true, force: true }));

describe('tape``', () => {
  test('removes the indentation every line shares, and escapes nothing', () => {
    const { source } = tape`
      Type "C:\new"
        Enter
    `;
    assert.equal(source, 'Type "C:\\new"\n  Enter\n');
  });

  test('interpolates values as they are', () => {
    const command = 'npm test';
    assert.equal(tape`Type "${command}"`.source, 'Type "npm test"\n');
  });
});

describe('binaryPath', () => {
  test('is DEMOGOD_BINARY when it is set', () => {
    assert.equal(binaryPath(), process.env.DEMOGOD_BINARY);
  });
});

describe('record', { skip }, () => {
  test('saves a tape file to its outputs, reporting as it goes', { skip: unixShell }, async () => {
    const file = path.join(directory, 'hello.tape');
    await writeFile(file, `${SMALL}Output hello.gif\nCaption "Hi" "there"\nType "echo hi"\nEnter\nWait\n`);
    const events = [];

    const saved = await record(file, { onEvent: (event) => events.push(event.type) });

    assert.equal(saved.length, 1);
    assert.equal(saved[0].path, path.join(directory, 'hello.gif'));
    assert.equal(saved[0].duration, 1.45);
    assert.ok(saved[0].frames > 5);
    assert.equal((await stat(saved[0].path)).size, saved[0].bytes);
    assert.deepEqual(events, ['scene', 'recorded', 'saved']);
  });

  test('records tape`` source, resolving paths from cwd', { skip: unixShell }, async () => {
    const saved = await record(tape`
      ${SMALL}
      Output from-source.cast
      Type "echo from-source"
      Enter
      Wait
    `, { cwd: directory });

    assert.equal(saved[0].path, path.join(directory, 'from-source.cast'));
    assert.match(await readFile(saved[0].path, 'utf8'), /from-source/);
  });

  test('runs JavaScript for `Do`, in order, between keystrokes', { skip: unixShell }, async () => {
    const calls = [];
    const saved = await record(
      tape`
        ${SMALL}
        Do first
        Type "cat note.txt"
        Enter
        Wait /written by JavaScript/
        Do second
      `,
      {
        cwd: directory,
        output: 'actions.png',
        actions: {
          first: async () => {
            await writeFile(path.join(directory, 'note.txt'), 'written by JavaScript\n');
            calls.push('first');
          },
          second: () => calls.push('second'),
        },
      },
    );

    assert.deepEqual(calls, ['first', 'second']);
    assert.equal(saved[0].frames, 1);
  });

  test('an action that throws fails the recording on its line', { skip: unixShell }, async () => {
    const recording = record(tape`
      ${SMALL}
      Type "x"
      Do explode
    `, { cwd: directory, output: 'never.gif', actions: { explode: () => { throw new Error('kaboom'); } } });

    await assert.rejects(recording, (error) => {
      assert.ok(error instanceof DemogodError);
      assert.equal(error.message, 'Do explode failed: kaboom');
      assert.equal(error.location.line, 7);
      return true;
    });
  });

  test('a tape error says where, and how to fix it', async () => {
    await assert.rejects(record(tape`Slep 1s`, { cwd: directory, output: 'x.gif' }), (error) => {
      assert.ok(error instanceof DemogodError);
      assert.equal(error.message, 'no such command: Slep');
      assert.equal(error.location.line, 1);
      assert.equal(error.help, 'did you mean Sleep?');
      assert.match(error.text, /--> .*:1/);
      return true;
    });
  });

  test('an onEvent that throws ends the recording with its error', { skip: unixShell }, async () => {
    const recording = record(tape`
      ${SMALL}
      Caption "One"
      Do later
      Type "x"
    `, {
      cwd: directory,
      output: 'thrown.png',
      actions: { later() {} },
      onEvent: () => {
        throw new Error('listener broke');
      },
    });

    await assert.rejects(recording, /listener broke/);
  });

  test('an error without a line has no location, not a null one', async () => {
    await assert.rejects(record(tape`Type "x"`, { cwd: directory, output: 'x.avi' }), (error) => {
      assert.equal(error.location, undefined);
      return true;
    });
  });

  test('a missing file is a DemogodError', async () => {
    await assert.rejects(record(path.join(directory, 'nope.tape')), DemogodError);
  });

  test('can be aborted', { skip: unixShell }, async () => {
    const controller = new AbortController();
    const recording = record(tape`
      ${SMALL}
      Type "sleep 10"
      Enter
      Sleep 10s
    `, { cwd: directory, output: 'aborted.gif', signal: controller.signal });
    setTimeout(() => controller.abort(new Error('stop')), 300);

    await assert.rejects(recording, /stop/);
  });

  test('rejects what is not a tape', async () => {
    await assert.rejects(record(42), TypeError);
  });
});

describe('check', { skip }, () => {
  test('counts scenes and steps without recording', async () => {
    const result = await check(tape`
      Caption "One"
      Type "a"
      Caption "Two"
      Type "b"
      Enter
    `, { cwd: directory });

    assert.deepEqual(result, { scenes: 2, steps: 3 });
  });

  test('knows the actions JavaScript provides', async () => {
    await assert.rejects(check(tape`Do reset`, { cwd: directory }), /no action is named reset/);
    assert.deepEqual(await check(tape`Do reset`, { cwd: directory, actions: { reset() {} } }), { scenes: 1, steps: 1 });
  });
});

describe('themes', { skip }, () => {
  test('lists the built-in themes', async () => {
    const names = await themes();
    assert.ok(names.includes('Dracula'));
    assert.ok(names.includes('Catppuccin Mocha'));
  });
});
