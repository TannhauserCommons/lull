// Runs the Worker as it deploys (wrangler's own bundle, its bindings from wrangler.jsonc) in Miniflare, Cloudflare's
// local runtime, with every outside service played by `net`: nothing leaves this machine.
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { convertV4MiniflareOptions, Miniflare } from 'miniflare';
import { unstable_getMiniflareWorkerOptions } from 'wrangler';

const dir = fileURLToPath(new URL('../', import.meta.url));
const out = '.wrangler/test';
/** The share host tests use until the owner has picked the real one (docs/adr/0034). */
export const HOST = 'https://lull.share-host.invalid';

let built = false;
function build() {
  if (built) return;
  const wrangler = fileURLToPath(new URL('../node_modules/wrangler/bin/wrangler.js', import.meta.url));
  execFileSync(process.execPath, [wrangler, 'deploy', '--dry-run', '--outdir', out], { cwd: dir, stdio: 'pipe' });
  built = true;
}

/** A fresh Worker (its Durable Objects empty) whose outside requests `net` answers. */
export async function start(net) {
  build();
  const { workerOptions } = unstable_getMiniflareWorkerOptions(dir + 'wrangler.jsonc');
  // The bundle is one module, so wrangler's rules for finding others have nothing to do (and Miniflare 5 refuses them).
  delete workerOptions.modulesRules;
  // wrangler hands Miniflare's older (v4) form of the options; Miniflare converts them.
  const mf = new Miniflare(convertV4MiniflareOptions({
    workers: [{
      ...workerOptions,
      rootPath: dir,
      modules: true,
      scriptPath: out + '/index.js',
      bindings: { LASTFM_API_KEY: 'test-key' },
      outboundService: (request) => net.answer(request),
      // Each Worker's Durable Objects start empty and live in memory.
      unsafeEphemeralDurableObjects: true,
    }],
  }));
  await mf.ready;
  return {
    /** `country` is where Cloudflare places the client (request.cf.country). */
    get: (path, { ip = '192.0.2.1', method = 'GET', country = 'US' } = {}) =>
      mf.dispatchFetch(HOST + path, { method, headers: { 'cf-connecting-ip': ip }, cf: { country }, redirect: 'manual' }),
    dispose: () => mf.dispose(),
  };
}

/**
 * The outside world: `on(pattern, answer)` adds a service (the newest match wins), and every request is kept in
 * `asked`, so a test can count the calls a service got. Anything not set up answers 404.
 */
export function internet() {
  const routes = [];
  const asked = [];
  return {
    asked,
    on(pattern, answer) { routes.unshift({ pattern, answer }); },
    count(pattern) { return asked.filter((u) => pattern.test(u)).length; },
    async answer(request) {
      asked.push(request.url);
      const route = routes.find((r) => r.pattern.test(request.url));
      return route ? route.answer(new URL(request.url), request) : new Response('not here', { status: 404 });
    },
  };
}

/** A picture as a CDN answers it; `tag` makes its bytes tell which source it came from. */
export function picture(tag) {
  return new Response(`picture:${tag}`, { headers: { 'content-type': 'image/jpeg' } });
}

export function json(body, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}
