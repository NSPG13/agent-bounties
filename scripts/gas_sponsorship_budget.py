"""Shared daily allowance for trusted legacy keepers; no key ever leaves the process."""
from __future__ import annotations
import hashlib
import json
import os
import time
import urllib.request

ENDPOINT = "https://api.agentbounties.app/v1/operator/gas-sponsorship/reservations"
ORACLE = "0x420000000000000000000000000000000000000f"
MAXIMUM_FEE_PER_GAS = 100_000_000
MAXIMUM_COST = 100_000_000_000_000

class BudgetUnavailable(RuntimeError):
    pass

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise BudgetUnavailable("Gas budget endpoint redirected; no transaction sent")

def reserve(client, private_key: str, gas_limit: int, contract: str, signature: str, args: tuple[str, ...], *, calldata=None) -> tuple[int, int]:
    token = os.environ.get("GAS_SPONSOR_BUDGET_TOKEN", "")
    if len(token) < 32:
        raise BudgetUnavailable("Shared gas sponsorship budget credential is not configured; no transaction sent")
    if client.chain_id() != 8453 or not 0 < gas_limit <= 1_500_000:
        raise BudgetUnavailable("Unsupported chain or gas limit; no transaction sent")
    # Force cast's fee cap to the exact reserved value. Its implicit dynamic fee
    # estimate previously could exceed the preflight gas-price sample.
    if calldata is None:
        calldata = client.run("calldata", signature, *args)
    if not calldata.startswith("0x") or len(calldata) < 10 or len(calldata) % 2:
        raise BudgetUnavailable("Could not encode exact transaction for the gas budget")
    try:
        bytes.fromhex(calldata[2:])
        l1 = int(client.call(ORACLE, "getL1FeeUpperBound(uint256)(uint256)", str((len(calldata)-2)//2+256), block="latest").split()[0],0)
        operator = int(client.call(ORACLE, "getOperatorFee(uint256)(uint256)", str(gas_limit), block="latest").split()[0],0)
    except (ValueError, IndexError) as error:
        raise BudgetUnavailable("Base gas quote is unavailable; no transaction sent") from error
    price = client.gas_price()
    if not 0 < price <= MAXIMUM_FEE_PER_GAS:
        raise BudgetUnavailable("Base gas price exceeds the keeper ceiling")
    fee_cap = min(MAXIMUM_FEE_PER_GAS, price * 2)
    maximum = gas_limit * fee_cap + 2*(l1+operator)
    if l1 < 0 or operator < 0 or not 0 < maximum <= MAXIMUM_COST:
        raise BudgetUnavailable("Total execution, L1 and operator gas exceeds the keeper cap")
    scope = {"chain_id":8453,"from":client.keeper_address(private_key),"to":contract.lower(),"data":calldata.lower(),"value":"0","gas_limit":gas_limit,"max_fee_per_gas":str(fee_cap)}
    digest = hashlib.sha256(json.dumps(scope,sort_keys=True,separators=(",",":")).encode()).hexdigest()
    body = {"network":"base-mainnet","relayer_address":scope["from"],"transaction_digest":digest,"maximum_fee_wei":str(maximum)}
    req = urllib.request.Request(ENDPOINT, data=json.dumps(body).encode(), headers={"Authorization":f"Bearer {token}","Content-Type":"application/json"}, method="POST")
    try:
        with urllib.request.build_opener(NoRedirect).open(req, timeout=min(8,client.remaining_command_timeout())) as response:
            result = json.loads(response.read(16385))
    except Exception as error:
        # Never echo the request, credential, response body or provider headers.
        raise BudgetUnavailable("Shared daily gas budget unavailable or exhausted; no transaction sent") from None
    if result.get("schema") != "agent-bounties/operator-gas-reservation-v1" or result.get("reserved") is not True or any(result.get(k)!=v for k,v in body.items()) or result.get("daily_limit_micro_usdc") != 1_000_000 or result.get("carries_forward") is not False or result.get("transaction_submitted") is not False:
        raise BudgetUnavailable("Gas reservation did not match this exact transaction; no transaction sent")
    valid_until=result.get("valid_until")
    if not isinstance(valid_until,int) or not time.time()+1 < valid_until <= time.time()+35:
        raise BudgetUnavailable("Gas reservation expired before broadcast; no transaction sent")
    lease_until=result.get("signer_lease_expires_at")
    if result.get("shared_signer_lease") is not True or not isinstance(lease_until,int) or isinstance(lease_until,bool) or lease_until != valid_until+15:
        raise BudgetUnavailable("Shared transaction sender lease is unavailable; no transaction sent")
    nonce=result.get("sender_nonce")
    if not isinstance(nonce,int) or isinstance(nonce,bool) or not 0 <= nonce <= 2**63-1:
        raise BudgetUnavailable("Sender nonce reservation is invalid; no transaction sent")
    return fee_cap, nonce


def send_cast(run, cast, rpc_url, private_key, gas_limit, contract, signature, *args,
              expected_nonce=None, calldata=None):
    """Budget existing standalone cast workflows without granting new authority.

    The caller must validate its exact action and signer before this boundary.
    The supplied runner keeps its existing secret-isolated environment. Quotes
    and sends have bounded timeouts; an unknown send is never retried here.
    """
    if (signature is None) != (calldata is not None) or (calldata is not None and args):
        raise BudgetUnavailable("Exactly one reviewed ABI call or calldata is required")
    payload = [contract, "--data", calldata] if calldata is not None else [contract, signature, *args]

    class Client:
        def run(self, *command):
            try:
                return run([str(cast), *command], timeout=8)
            except Exception:
                raise BudgetUnavailable("Gas preflight unavailable; no transaction sent") from None

        def chain_id(self):
            return int(self.run("chain-id", "--rpc-url", rpc_url).split()[0], 0)

        def keeper_address(self, key):
            address = self.run("wallet", "address", "--private-key", key).strip().lower()
            if len(address) != 42 or not address.startswith("0x"):
                raise BudgetUnavailable("Invalid gas sponsor address")
            try:
                bytes.fromhex(address[2:])
            except ValueError:
                raise BudgetUnavailable("Invalid gas sponsor address") from None
            return address

        def gas_price(self):
            return int(self.run("gas-price", "--rpc-url", rpc_url).split()[0], 0)

        def call(self, target, abi, *values, block):
            return self.run("call", target, abi, *values, "--rpc-url", rpc_url, "--block", block)

        def remaining_command_timeout(self):
            return 8

    try:
        client = Client()
        if len(os.environ.get("GAS_SPONSOR_BUDGET_TOKEN", "")) < 32:
            raise BudgetUnavailable("Shared gas sponsorship budget credential is not configured; no transaction sent")
        if client.chain_id() != 8453 or not 0 < gas_limit <= 1_500_000:
            raise BudgetUnavailable("Unsupported chain or gas limit; no transaction sent")
        sender = client.keeper_address(private_key)
        client.run("call", *payload, "--from", sender, "--rpc-url", rpc_url)
        estimate = int(client.run("estimate", *payload,
                                  "--from", sender, "--rpc-url", rpc_url).split()[0], 0)
        if not 0 < estimate <= gas_limit:
            raise BudgetUnavailable("Exact call exceeds the sponsor gas limit; no transaction sent")
        fee_cap, nonce = reserve(client, private_key, gas_limit, contract, signature, args,
                                 **({"calldata": calldata} if calldata is not None else {}))
    except BudgetUnavailable:
        raise
    except Exception:
        raise BudgetUnavailable("Gas preflight unavailable; no transaction sent") from None
    if expected_nonce is not None and nonce != expected_nonce:
        raise BudgetUnavailable("Reserved nonce differs from the reviewed nonce; no transaction sent")
    try:
        return run([str(cast), "send", "--json", "--rpc-url", rpc_url,
                    "--chain", "8453", "--nonce", str(nonce), "--gas-limit", str(gas_limit),
                    "--gas-price", str(fee_cap), "--priority-gas-price", str(min(1_000_000, fee_cap)),
                    "--private-key", private_key, *payload], timeout=10)
    except Exception:
        # A timeout may have occurred after broadcast. Preserve the reservation;
        # the next run must reconcile canonical action state and the nonce fence.
        raise BudgetUnavailable("Sponsor send outcome unknown; reconcile before retrying") from None
