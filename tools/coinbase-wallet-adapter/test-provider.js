import assert from "node:assert/strict";
import { accountFromUser, createAuthenticatedProvider, userRejected } from "./provider.js";

const account = "0x1234567890abcdef1234567890abcdef12345678";
let signedIn = false;
let authCalls = 0;
let loadCalls = 0;
const requests = [];
const attached = [];
const underlying = {
  async request(request) {
    requests.push(request);
    if (request.method === "eth_accounts" || request.method === "eth_requestAccounts") return [account];
    return `result:${request.method}`;
  },
  on(event, handler) {
    attached.push({ event, handler });
  },
  removeListener() {},
};

const provider = createAuthenticatedProvider({
  chainIdHex: "0x2105",
  async loadProvider() {
    loadCalls += 1;
    return underlying;
  },
  async isAuthenticated() {
    return signedIn;
  },
  async ensureAuthenticated() {
    authCalls += 1;
    signedIn = true;
    return account;
  },
});

assert.equal(await provider.request({ method: "eth_chainId" }), "0x2105");
assert.deepEqual(await provider.request({ method: "eth_accounts" }), []);
assert.equal(loadCalls, 0, "read-only account discovery must not initialize a provider for a signed-out user");
assert.equal(await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: "0x2105" }] }), null);
assert.equal(loadCalls, 0, "Base chain acknowledgement must remain lazy");

const accountListener = () => {};
provider.on("accountsChanged", accountListener);
assert.deepEqual(await provider.request({ method: "eth_requestAccounts" }), [account]);
assert.equal(authCalls, 1);
assert.equal(loadCalls, 1);
assert.equal(attached.length, 1);
assert.equal(attached[0].event, "accountsChanged");

assert.equal(
  await provider.request({ method: "eth_signTypedData_v4", params: [account, "{}"] }),
  "result:eth_signTypedData_v4",
);
assert.equal(authCalls, 2, "every signature request must re-check the authenticated session");
assert.equal(loadCalls, 1, "the underlying provider must be initialized once");
assert.deepEqual(requests.map((request) => request.method), ["eth_requestAccounts", "eth_signTypedData_v4"]);

assert.equal(accountFromUser({ evmAccountObjects: [{ address: account }] }), account);
assert.equal(accountFromUser({ evmAccounts: [account.toUpperCase().replace("0X", "0x")] }), account);
assert.equal(accountFromUser({}), null);

const rejection = userRejected("No");
assert.equal(rejection.code, 4001);
assert.equal(rejection.message, "No");

console.log("Coinbase adapter core preserves lazy discovery, Base-only routing, and explicit authentication");
