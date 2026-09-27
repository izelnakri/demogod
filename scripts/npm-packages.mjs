#!/usr/bin/env node
// Builds the npm packages from the release archives CI made:
//
//   node scripts/npm-packages.mjs <archives-dir> <out-dir>      (e.g. artifacts dist/npm)
//
// Writes <out-dir>/demogod-<platform>/ for every platform — package.json, bin/demogod, README —
// and copies npm/ to <out-dir>/demogod with the README and LICENSE beside it. Every version is
// Cargo.toml's, and a mismatch in npm/package.json fails here rather than on the registry.
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const [archives, out] = process.argv.slice(2);
if (!archives || !out) {
  console.error('usage: node scripts/npm-packages.mjs <archives-dir> <out-dir>');
  process.exit(2);
}

const root = path.join(import.meta.dirname, '..');
if (!path.relative(path.join(root, 'npm'), path.resolve(out)).startsWith('..')) {
  console.error(`${out} is inside npm/, which is copied into it: write the packages somewhere else, e.g. dist/npm`);
  process.exit(2);
}
const version = readFileSync(path.join(root, 'Cargo.toml'), 'utf8').match(/^version = "(.+)"/m)[1];
const main = JSON.parse(readFileSync(path.join(root, 'npm/package.json'), 'utf8'));

/** Every Rust target the release builds, and the npm platform it is published as. */
const TARGETS = {
  'x86_64-unknown-linux-musl': ['linux', 'x64'],
  'aarch64-unknown-linux-musl': ['linux', 'arm64'],
  'x86_64-apple-darwin': ['darwin', 'x64'],
  'aarch64-apple-darwin': ['darwin', 'arm64'],
  'x86_64-pc-windows-msvc': ['win32', 'x64'],
  'aarch64-pc-windows-msvc': ['win32', 'arm64'],
};

const mismatched = [main.version, ...Object.values(main.optionalDependencies)].filter((v) => v !== version);
if (mismatched.length > 0) {
  console.error(`npm/package.json has versions ${mismatched.join(', ')}, and Cargo.toml has ${version}: run make release`);
  process.exit(1);
}

rmSync(out, { recursive: true, force: true });
for (const [target, [os, cpu]] of Object.entries(TARGETS)) {
  const windows = os === 'win32';
  const archive = path.join(archives, `demogod-${target}${windows ? '.zip' : '.tar.gz'}`);
  if (!existsSync(archive)) {
    console.error(`missing ${archive}`);
    process.exit(1);
  }
  const unpacked = mkdtempSync(path.join(tmpdir(), 'demogod-npm-'));
  execFileSync(windows ? 'unzip' : 'tar', windows ? ['-q', archive, '-d', unpacked] : ['-xzf', archive, '-C', unpacked]);

  const name = `demogod-${os}-${cpu}`;
  const binary = `demogod${windows ? '.exe' : ''}`;
  const directory = path.join(out, name);
  mkdirSync(path.join(directory, 'bin'), { recursive: true });
  cpSync(path.join(unpacked, `demogod-${target}`, binary), path.join(directory, 'bin', binary));
  chmodSync(path.join(directory, 'bin', binary), 0o755);
  rmSync(unpacked, { recursive: true, force: true });
  writeFileSync(
    path.join(directory, 'package.json'),
    JSON.stringify(
      {
        name,
        version,
        description: `The demogod binary for ${os} ${cpu}. Install demogod, which depends on it.`,
        repository: main.repository,
        homepage: main.homepage,
        license: main.license,
        os: [os],
        cpu: [cpu],
        files: ['bin/'],
        preferUnplugged: true,
      },
      null,
      2,
    ) + '\n',
  );
  writeFileSync(path.join(directory, 'README.md'), `# ${name}\n\nThe \`demogod\` binary for ${os} ${cpu}. Install [demogod](https://www.npmjs.com/package/demogod) instead, which depends on this.\n`);
  console.log(`${name}@${version}`);
}

const mainDirectory = path.join(out, 'demogod');
cpSync(path.join(root, 'npm'), mainDirectory, { recursive: true, filter: (source) => !/node_modules|platforms|[/\\]test$/.test(source) });
cpSync(path.join(root, 'README.md'), path.join(mainDirectory, 'README.md'));
cpSync(path.join(root, 'LICENSE'), path.join(mainDirectory, 'LICENSE'));
main.files.push('README.md', 'LICENSE');
delete main.scripts;
writeFileSync(path.join(mainDirectory, 'package.json'), JSON.stringify(main, null, 2) + '\n');
console.log(`demogod@${version}`);
