// Coinbase signs immediately after an SDK request. The app must supply the
// human review that an extension wallet would otherwise provide.
const ADDRESS = /^0x[0-9a-f]{40}$/i;
const error = (message, code = 4100) => Object.assign(new Error(message), { code });
const copy = value => JSON.parse(JSON.stringify(value, (_, item) => typeof item === "bigint" ? item.toString() : item));

export function createPostingRequests({ getProvider, currentAddress, readiness, confirm }) {
  let pending = false;
  const attempted = new Set();
  return async function request(input) {
    if (pending) throw error("Finish the open wallet review first.", -32002);
    // Freeze both the request and its explanation before any asynchronous work.
    const beforeSubmit = input.agentBountiesBeforeSubmit;
    if (beforeSubmit !== undefined && typeof beforeSubmit !== "function") throw error("Invalid posting checkpoint.");
    const args = copy(input), context = args.agentBountiesPostingContext;
    if (!context || !readiness()) throw error("Open the saved bounty's funding review before using this wallet to post.");
    pending = true;
    try {
      const wallet = await currentAddress();
      if (!ADDRESS.test(wallet || "") || context.creatorAddress?.toLowerCase() !== wallet.toLowerCase()) throw error("The posting wallet changed. Review the saved bounty again.");
      const sdk = await getProvider(), checks = readiness();
      let params, summary, label, title, expiresAt, details, note;
      if (args.method === "eth_sendTransaction") {
        const tx = args.params?.[0];
        if (args.params?.length !== 1 || !tx || tx.from?.toLowerCase() !== wallet.toLowerCase()
          || (tx.chainId !== undefined && BigInt(tx.chainId) !== 8453n)
          || Object.keys(tx).some(key => !["from", "to", "data", "value", "chainId"].includes(key))) throw error("The Base posting transaction is invalid.");
        const description = checks.describeWalletCalls({ calls: [tx], context });
        expiresAt = description.calls[0].expiresAt;
        const fees = await checks.estimateFees({ provider: sdk, wallet, calls: [tx] });
        if (fees.estimatedTotalWei == null) throw error("The full Base network fee is unavailable. Nothing was sent; keep this bounty saved and check again.");
        const balance = await sdk.request({ method: "eth_getBalance", params: [wallet, fees.blockNumber || "latest"] });
        if (typeof balance !== "string" || !/^0x[0-9a-f]+$/i.test(balance)) throw error("The Base ETH balance could not be checked. Nothing was sent.");
        if (BigInt(balance) < fees.estimatedTotalWei) throw error(`Not enough Base ETH for the estimated network fee. Add at least ${checks.formatUnits(fees.estimatedTotalWei - BigInt(balance), 18, 18)} ETH. Nothing was sent; keep this saved operation.`);
        params = [{ ...tx, chainId: "0x2105" }];
        summary = `${description.summary}\nEstimated network fee: ${checks.formatUnits(fees.estimatedTotalWei, 18, 18)} ETH, paid by you. This includes Base execution and data fees. The final fee can change. No gas sponsorship.\nFrom: ${wallet}\nContract: ${tx.to}`;
        title = "Confirm bounty transaction";
        label = "Send transaction";
        const call = description.calls[0];
        details = [
          ["Action", ({creation:"Create and fund this bounty",allowance:"Approve a spending limit",funding:"Fund this bounty",terms_publication:"Publish bounty terms"})[call.kind]],
          ["Network", "Base (chain 8453)"],
          ["Amount", `${checks.formatUnits(call.amountUsdcUnits)} Base USDC`],
          [call.kind === "allowance" ? "Spender" : "Recipient", call.recipient || call.spender || tx.to],
          [call.kind === "creation" ? "Factory contract" : "Transaction contract", tx.to],
          ["Estimated network fee", `${checks.formatUnits(fees.estimatedTotalWei, 18, 18)} ETH (paid by you)`],
          ["Expiry", expiresAt || "No automatic expiry"],
          ["From wallet", wallet],
        ];
        note = "The network fee can change. No gas sponsorship.";
      } else if (args.method === "eth_signTypedData_v4") {
        if (args.params?.length !== 2 || args.params[0]?.toLowerCase() !== wallet.toLowerCase()) throw error("The signing wallet changed.");
        const typedData = typeof args.params[1] === "string" ? JSON.parse(args.params[1]) : args.params[1];
        const authorization = checks.validateFundingAuthorization({ typedData, context });
        expiresAt = authorization.expiresAt;
        params = [wallet, authorization.serialized];
        summary = authorization.summary;
        title = "Approve bounty funding signature";
        label = "Sign funding authorization";
        details = [["Action", "Authorize this bounty to receive funds"], ["Network", "Base (chain 8453)"], ["Amount", `${checks.formatUnits(authorization.amountUsdcUnits)} Base USDC`], ["Recipient", authorization.to], ["Expiry", authorization.expiresAt], ["Signature fee", "No network fee"], ["From wallet", wallet]];
        note = "This signature alone does not fund the bounty. Next, review the creation transaction and its network fee.";
      } else throw error("This posting request is unsupported.", 4200);
      const key = JSON.stringify([context.bountyAddress?.toLowerCase(), args.method, params]);
      if (attempted.has(key)) throw error("This wallet request was already sent. Check the same bounty's status; do not send it again.");
      await confirm({ title, label, summary, details, note });
      if ((await currentAddress())?.toLowerCase() !== wallet.toLowerCase()) throw error("The wallet changed during review. Nothing was sent.");
      if (expiresAt && Date.parse(expiresAt) <= Date.now()) throw error("The funding request expired during review. Nothing was sent.");
      // A cancelled review must not create an irreversible account checkpoint.
      // Once approved, persist the signing intent before the SDK can act.
      await beforeSubmit?.();
      if ((await currentAddress())?.toLowerCase() !== wallet.toLowerCase()) throw error("The wallet changed while saving the request. Nothing was sent.");
      if (expiresAt && Date.parse(expiresAt) <= Date.now()) throw error("The funding request expired while saving. Nothing was sent.");
      // Once passed to CDP, even a rejected/lost SDK reply may be uncertain.
      // The composer's durable journal retains the same operation across reloads.
      attempted.add(key);
      try { return await sdk.request({ method: args.method, params }); }
      catch (_) { throw error("Coinbase did not return a confirmed result. The request may have been submitted. Check this saved bounty before taking another wallet action.", -32000); }
    } finally { pending = false; }
  };
}
