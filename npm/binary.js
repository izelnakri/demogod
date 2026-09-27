// Where the native demogod binary is: the platform package npm installed beside this one.
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

/** The platforms a binary is published for, as `${process.platform}-${process.arch}`. */
export const PLATFORMS = ['darwin-arm64', 'darwin-x64', 'linux-arm64', 'linux-x64', 'win32-arm64', 'win32-x64'];

/**
 * The path of the demogod binary for this machine.
 *
 * `DEMOGOD_BINARY` overrides it, for a binary built from source: `cargo build --release`, then
 * `DEMOGOD_BINARY=target/release/demogod`.
 *
 * @returns {string}
 */
export function binaryPath() {
  if (process.env.DEMOGOD_BINARY) return process.env.DEMOGOD_BINARY;

  const platform = `${process.platform}-${process.arch}`;
  const executable = process.platform === 'win32' ? 'demogod.exe' : 'demogod';
  if (!PLATFORMS.includes(platform)) {
    throw new Error(
      `demogod has no prebuilt binary for ${platform}. Build one with \`cargo install demogod\` ` +
        'and point DEMOGOD_BINARY at it.',
    );
  }
  try {
    return require.resolve(`demogod-${platform}/bin/${executable}`);
  } catch {
    throw new Error(
      `demogod-${platform} is not installed, so there is no binary to run. It is an optional ` +
        'dependency of demogod: reinstall without --no-optional / --omit=optional, or set DEMOGOD_BINARY.',
    );
  }
}
