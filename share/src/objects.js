// The Worker's two Durable Objects (docs/adr/0034, "A burst of clicks costs one lookup"):
// - Cover, one per album or artist worldwide: runs the lookup once however many ask at once, and keeps the address
//   of the picture it found (never its bytes);
// - Gate, one for the Worker: hands out turns of each outside service's budget, worldwide.

import { DurableObject } from 'cloudflare:workers';
import { find } from './sources.js';

const DAY = 24 * 3600 * 1000;
/** A picture found is looked for again after a month; "none" after a day; a lookup a source couldn't answer (often a
    moment's slowness) after a minute. The gate keeps the retries polite. */
const KEEP_FOUND = 30 * DAY;
const KEEP_NONE = DAY;
const RETRY = 60 * 1000;

export class Cover extends DurableObject {
  /** The one lookup running, which every request arriving meanwhile waits on. */
  running = null;

  /**
   * The picture for `subject`: `{ url, source, until }`, `url` null when no source has one (yet).
   * `broken` is an address the caller found no longer answering: it is dropped, and the lookup runs again.
   */
  async picture(subject, broken = null) {
    let kept = await this.ctx.storage.get('picture');
    if (kept && broken && kept.url === broken) {
      await this.ctx.storage.delete('picture');
      kept = null;
    }
    if (kept && Date.now() < kept.until) return kept;
    if (kept?.url) {
      // Stale: answered at once, and looked for again behind it.
      this.ctx.waitUntil(this.lookup(subject));
      return kept;
    }
    return this.lookup(subject);
  }

  lookup(subject) {
    this.running ??= this.run(subject).finally(() => { this.running = null; });
    return this.running;
  }

  async run(subject) {
    const found = await find(subject, { gate: gate(this.env), lastfmKey: this.env.LASTFM_API_KEY });
    const now = Date.now();
    const kept = await this.ctx.storage.get('picture');
    let next;
    if (found.url) next = { url: found.url, source: found.source, until: now + KEEP_FOUND };
    // A picture kept still answered when it was last served: nothing found now doesn't replace it.
    else if (kept?.url) next = { ...kept, until: now + RETRY };
    else next = { url: null, source: null, until: now + (found.busy ? RETRY : KEEP_NONE) };
    await this.ctx.storage.put('picture', next);
    return next;
  }
}

/** Each service's budget: calls per window (ms). Polite whatever the traffic: over it, a source is skipped. */
const BUDGETS = {
  itunes: [10, 60 * 1000],
  deezer: [10, 1000],
  musicbrainz: [1, 1000],
  lastfm: [2, 1000],
  wikidata: [1, 1000],
};
/** A 429 or 403 stops a service this long. */
const PAUSE = 10 * 60 * 1000;
/** A turn that frees up within this long is waited for rather than skipped. */
const WAIT = 1000;

export class Gate extends DurableObject {
  calls = new Map();
  paused = new Map();

  /** A turn of `service`'s budget: true when the call may be made now. */
  async take(service) {
    const [limit, window] = budget(service);
    for (;;) {
      const now = Date.now();
      if ((this.paused.get(service) || 0) > now) return false;
      const recent = (this.calls.get(service) || []).filter((t) => t > now - window);
      if (recent.length < limit) {
        recent.push(now);
        this.calls.set(service, recent);
        return true;
      }
      const free = recent[0] + window - now;
      if (free > WAIT) return false;
      this.calls.set(service, recent);
      await new Promise((resolve) => setTimeout(resolve, free));
    }
  }

  pause(service) {
    budget(service);
    this.paused.set(service, Date.now() + PAUSE);
  }
}

function budget(service) {
  const b = BUDGETS[service];
  if (!b) throw new Error(`no budget for ${service}`);
  return b;
}

/** The Worker's one gate. */
export function gate(env) {
  return env.GATE.get(env.GATE.idFromName('gate'));
}
