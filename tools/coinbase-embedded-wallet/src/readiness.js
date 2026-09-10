// Read-only readiness checks. A timeout never retries authentication or a wallet request.
export function createReadinessGate(check, { timeoutMs = 12000, cooldownMs = 60000, now = Date.now, setTimer = setTimeout, clearTimer = clearTimeout } = {}) {
  let pending = null;
  let failures = 0;
  let retryAt = 0;
  let ready = false;
  let state = "unchecked";
  const snapshot = () => Object.freeze({ state, consecutiveFailures: failures, retryAt });
  const unavailable = (code, message) => Object.assign(new Error(message), { code });
  async function run() {
    if (ready) return snapshot();
    if (pending) return pending;
    if (retryAt > now()) throw unavailable("coinbase_startup_paused", "Coinbase is still unavailable. Retry in a minute, open Agent Bounties in your phone browser, or choose another wallet. Your draft is saved.");
    state = "checking";
    pending = new Promise((resolve, reject) => {
      let settled = false;
      const finish = (error) => {
        if (settled) return;
        settled = true;
        clearTimer(timer);
        if (error) {
          failures += 1;
          retryAt = failures >= 2 ? now() + cooldownMs : 0;
          state = retryAt ? "paused" : "unavailable";
          reject(unavailable("coinbase_startup_unavailable", "Coinbase wallet configuration could not load. Retry, open Agent Bounties in your phone browser, or choose another wallet. Your draft is saved."));
        } else {
          ready = true;
          failures = 0;
          retryAt = 0;
          state = "ready";
          resolve(snapshot());
        }
      };
      const timer = setTimer(() => finish(new Error("timeout")), timeoutMs);
      Promise.resolve().then(check).then(() => finish(), finish);
    }).finally(() => { pending = null; });
    return pending;
  }
  return Object.freeze({ run, snapshot });
}
