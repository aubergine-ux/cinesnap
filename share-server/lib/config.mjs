import path from 'node:path';

const truthy = new Set(['1', 'true', 'yes', 'on']);

export function loadConfig(env = process.env) {
  const port = Number.parseInt(env.PORT ?? '8080', 10);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`PORT is not a valid port: ${env.PORT}`);
  }

  const maxMb = Number(env.MAX_UPLOAD_MB ?? '500');
  if (!Number.isFinite(maxMb) || maxMb <= 0) {
    throw new Error(`MAX_UPLOAD_MB must be a positive number: ${env.MAX_UPLOAD_MB}`);
  }

  let publicUrl = (env.PUBLIC_URL ?? '').trim().replace(/\/+$/, '');
  if (publicUrl) {
    let parsed;
    try {
      parsed = new URL(publicUrl);
    } catch {
      throw new Error(`PUBLIC_URL is not a valid URL: ${env.PUBLIC_URL}`);
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      throw new Error('PUBLIC_URL must start with http:// or https://');
    }
    publicUrl = parsed.origin;
  }

  // TRUST_PROXY=1/true means one proxy in front; a larger number means that many hops.
  const rawProxy = (env.TRUST_PROXY ?? '').trim().toLowerCase();
  let trustProxy = 0;
  if (/^\d+$/.test(rawProxy)) trustProxy = Number(rawProxy);
  else if (truthy.has(rawProxy)) trustProxy = 1;

  return {
    port,
    dataDir: path.resolve(env.DATA_DIR || './data'),
    uploadToken: (env.UPLOAD_TOKEN ?? '').trim(),
    publicUrl,
    maxUploadBytes: Math.floor(maxMb * 1024 * 1024),
    trustProxy,
  };
}
