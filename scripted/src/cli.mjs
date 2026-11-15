import fs from 'node:fs';
import { parseArgs } from 'node:util';
import { ConfigError, load } from './config.mjs';
import { record } from './run.mjs';

const USAGE = `Record scripted browser demos as Cinesnap recordings.

Usage:
  cinesnap-script run <script.yml> [-o demo.cinesnap] [--render out.gif] [options]
  cinesnap-script check <script.yml>

Options:
  -o, --out <dir>       where to write the .cinesnap bundle
                        (default: <script name>.cinesnap, or a temporary one with --render)
      --render <file>   also render the recording with the Cinesnap CLI (.mp4 or .gif)
      --preset <name>   render preset, passed to the Cinesnap CLI (for example: readme)
      --keep            with --render and no -o: keep the temporary bundle
      --headed          show the browser window while recording
      --timeout <ms>    how long a step may wait for its element (default 15000)
      --cinesnap <path> the cinesnap or cinesnap-cli binary
                        (default: $CINESNAP_BIN, then cinesnap-cli or cinesnap on PATH)
      --ffmpeg <path>   the ffmpeg binary (default: $CINESNAP_FFMPEG, then PATH)
  -h, --help
  -V, --version

Set CINESNAP_SCRIPT_CHROMIUM to record with a specific Chromium or Chrome binary.`;

class UsageError extends Error {}

export async function main(argv) {
  let parsed;
  try {
    parsed = parseArgs({
      args: argv,
      allowPositionals: true,
      options: {
        out: { type: 'string', short: 'o' },
        render: { type: 'string' },
        preset: { type: 'string' },
        keep: { type: 'boolean' },
        headed: { type: 'boolean' },
        timeout: { type: 'string' },
        cinesnap: { type: 'string' },
        ffmpeg: { type: 'string' },
        help: { type: 'boolean', short: 'h' },
        version: { type: 'boolean', short: 'V' },
      },
    });
  } catch (e) {
    console.error(`cinesnap-script: ${e.message}\n\n${USAGE}`);
    return 2;
  }
  const { values, positionals } = parsed;
  if (values.help || (positionals.length === 0 && !values.version)) {
    console.log(USAGE);
    return values.help ? 0 : 2;
  }
  if (values.version) {
    console.log(JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version);
    return 0;
  }

  try {
    const [command, script, ...rest] = positionals;
    if (command !== 'run' && command !== 'check') throw new UsageError(`unknown command "${command}"`);
    if (!script) throw new UsageError(`${command} needs a script file`);
    if (rest.length) throw new UsageError(`unexpected argument "${rest[0]}"`);

    if (command === 'check') {
      const cfg = load(script);
      console.log(`${script}: ok (${cfg.steps.length} steps)`);
      return 0;
    }

    if (values.preset && !values.render) throw new UsageError('--preset only applies with --render');
    if (values.keep && !values.render) throw new UsageError('--keep only applies with --render');
    let timeout;
    if (values.timeout !== undefined) {
      timeout = Number(values.timeout);
      if (!Number.isFinite(timeout) || timeout <= 0) throw new UsageError('--timeout must be a positive number of milliseconds');
    }
    return await record({ script, ...values, timeout });
  } catch (e) {
    if (e instanceof UsageError) {
      console.error(`cinesnap-script: ${e.message}\n\n${USAGE}`);
      return 2;
    }
    console.error(`cinesnap-script: ${e instanceof ConfigError ? e.message : (e?.message ?? e)}`);
    return 1;
  }
}
