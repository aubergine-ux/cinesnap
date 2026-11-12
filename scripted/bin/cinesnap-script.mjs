#!/usr/bin/env node
import { main } from '../src/cli.mjs';

main(process.argv.slice(2)).then(
  (code) => process.exit(code),
  (err) => {
    console.error(`cinesnap-script: ${err?.message ?? err}`);
    process.exit(1);
  },
);
