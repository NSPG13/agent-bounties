import { AgentBountiesClient } from "./index.js";

/** Fixture credentials must never be sent to an arbitrary SDK example URL. */
export function localFixtureClient(
  baseUrl: string,
  credential: string | undefined,
  kind: "operator" | "wallet" = "operator",
): AgentBountiesClient {
  const url = new URL(baseUrl);
  if (
    url.protocol !== "http:" ||
    !["127.0.0.1", "localhost"].includes(url.hostname) ||
    url.username !== "" ||
    url.password !== "" ||
    url.pathname !== "/" ||
    url.search !== "" ||
    url.hash !== ""
  ) {
    throw new Error("Fixture smoke requires an http://127.0.0.1 or http://localhost base URL");
  }
  if (!credential?.trim()) {
    throw new Error(`${kind === "wallet" ? "AGENT_BOUNTIES_SMOKE_WALLET_SESSION" : "OPERATOR_API_TOKEN"} is required for intentional local fixture writes`);
  }
  return new AgentBountiesClient({
    baseUrl: url.origin,
    ...(kind === "wallet" ? { sessionToken: credential } : { operatorApiToken: credential }),
  });
}
