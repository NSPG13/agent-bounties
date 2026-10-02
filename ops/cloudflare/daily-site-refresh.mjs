// Secret hook is scoped to this Pages project. HTTP visitors cannot trigger it.
export default {
  async fetch() {
    return new Response("Not found", { status: 404 });
  },
  async scheduled(_controller, env) {
    const url = new URL(env.PAGES_DEPLOY_HOOK);
    if (url.origin !== "https://api.cloudflare.com" ||
        !/^\/client\/v4\/pages\/webhooks\/deploy_hooks\/[a-zA-Z0-9-]+$/.test(url.pathname) ||
        url.search || url.hash || url.username || url.password) {
      throw new Error("Invalid Pages rebuild hook configuration");
    }
    const response = await fetch(url, {
      // Workers supports manual redirects; non-2xx responses fail below.
      method: "POST", redirect: "manual", signal: AbortSignal.timeout(10000),
    });
    if (!response.ok) throw new Error("Daily Pages rebuild request failed");
    console.log("daily_site_refresh_requested");
  },
};
