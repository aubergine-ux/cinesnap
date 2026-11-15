import { createApp } from './lib/app.mjs';
import { loadConfig } from './lib/config.mjs';

let config;
try {
  config = loadConfig();
} catch (err) {
  console.error(`config error: ${err.message}`);
  process.exit(1);
}

const app = createApp(config);

app.server.listen(config.port, () => {
  console.log(`cinesnap-share listening on port ${config.port}, data in ${config.dataDir}`);
  if (config.publicUrl) console.log(`public url: ${config.publicUrl}`);
  if (!config.uploadToken) {
    console.log('UPLOAD_TOKEN is not set, uploads are refused until it is.');
  }
});

app.server.on('error', (err) => {
  console.error(`server error: ${err.message}`);
  process.exit(1);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    app.close().then(() => process.exit(0));
  });
}
