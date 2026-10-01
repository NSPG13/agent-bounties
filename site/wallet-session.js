/* A short wallet login is separate from payment authority. Tokens stay in memory. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.AgentBountiesWalletSession = api;
})(typeof window === "undefined" ? globalThis : window, function () {
  "use strict";
  const sessions = new WeakMap();
  const statement = "Sign in to Agent Bounties for 15 minutes. This does not link a website account or authorize a transaction, token approval, or payment.";
  function validateChallenge(value, api, address, now = Date.now()) {
    const origin = new URL(api), lines = String(value?.message || "").split("\n");
    if (!/^https:\/\/api\.agentbounties\.app$/.test(api) && !/^http:\/\/127\.0\.0\.1:3000$/.test(api)) throw new Error("Unsupported wallet login destination.");
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value?.challenge_id) || value.chain_id !== 8453 || value.address?.toLowerCase() !== address
      || lines.length !== 11 || lines[0] !== `${origin.host} wants you to sign in with your Ethereum account:`
      || lines[1].toLowerCase() !== address || lines[2] !== "" || lines[3] !== statement || lines[4] !== ""
      || lines[5] !== `URI: ${api}/v1/auth/wallet/session` || lines[6] !== "Version: 1" || lines[7] !== "Chain ID: 8453"
      || !/^Nonce: [0-9a-f]{64}$/.test(lines[8])) throw new Error("The wallet login message has a different identity or purpose. No signature was requested.");
    const challengeExpiry = Date.parse(value.expires_at);
    const issued = Date.parse(lines[9].replace(/^Issued At: /, "")), expires = Date.parse(lines[10].replace(/^Expiration Time: /, ""));
    // The SIWE message has eleven lines; tolerate no extra authority or resources.
    if (!Number.isFinite(issued) || !Number.isFinite(expires) || issued > now + 30_000 || now - issued > 300_000
      || expires - issued !== 900_000 || !Number.isFinite(challengeExpiry) || challengeExpiry <= now || challengeExpiry > now + 330_000) throw new Error("The wallet login has expired or changed. Retry to get a fresh ownership-only message.");
    return value.message;
  }
  async function authenticate(win, provider, address, request, api) {
    address = String(address).toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(address)) throw new Error("Connect a valid wallet to sign in.");
    const assertWallet = async () => {
      if (String((await provider.request({method:"eth_accounts"}))[0]).toLowerCase() !== address
        || String(await provider.request({method:"eth_chainId"})).toLowerCase() !== "0x2105") throw new Error("Your wallet changed. Reconnect the reviewed wallet on Base.");
    };
    await assertWallet();
    const prior = sessions.get(win);
    if (prior?.address === address && prior.expires > Date.now() + 30_000) return prior.token;
    const challenge = await request("/v1/auth/wallet/challenge", {address,chain_id:8453});
    const message = validateChallenge(challenge, api, address);
    await assertWallet();
    const encoded = "0x" + Array.from(new TextEncoder().encode(message), byte => byte.toString(16).padStart(2,"0")).join("");
    const signature = await provider.request({method:"personal_sign",params:[encoded,address]});
    await assertWallet();
    const result = await request("/v1/auth/wallet/session", {challenge_id:challenge.challenge_id,chain_id:8453,signature});
    const expires = Date.parse(result.expires_at);
    if (result.token_type !== "Bearer" || !/^abws_[0-9a-f]{64}$/.test(result.token) || result.principal?.kind !== "wallet"
      || result.principal.chain_id !== 8453 || result.principal.address?.toLowerCase() !== address
      || !Number.isFinite(expires) || expires <= Date.now() || expires > Date.now() + 930_000) throw new Error("The wallet login reply did not match this wallet.");
    await assertWallet();
    sessions.set(win, {address,token:result.token,expires});
    return result.token;
  }
  function clear(win) { sessions.delete(win); }
  return {validateChallenge,authenticate,clear};
});
