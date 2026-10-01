#!/usr/bin/env python3
"""Budget the exact zero-ETH-value call already authorized by a keeper workflow.

This wrapper adds no event or wallet authority. Its caller still validates the
action. Missing capacity, failed simulation and unknown outcomes stop the run.
"""
import argparse
import os
import subprocess
import sys
import gas_sponsorship_budget as budget


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--key-env", required=True)
    parser.add_argument("--gas-limit", type=int, required=True)
    parser.add_argument("target")
    parser.add_argument("signature")
    parser.add_argument("arguments", nargs="*")
    args = parser.parse_args()
    key = os.environ.get(args.key_env, "")
    if not key:
        parser.error("Configured keeper key is unavailable")
    environment = {k:v for k,v in os.environ.items() if k not in {args.key_env, "GAS_SPONSOR_BUDGET_TOKEN"}}

    def run(command, *, timeout):
        result = subprocess.run(command, env=environment, capture_output=True, text=True,
                                timeout=timeout, check=False)
        if result.returncode != 0:
            raise RuntimeError("Cast failed")
        return result.stdout.strip()

    try:
        result = budget.send_cast(run, "cast", args.rpc_url, key, args.gas_limit,
                                  args.target, args.signature, *args.arguments)
    except budget.BudgetUnavailable as error:
        print(str(error), file=sys.stderr)
        return 1
    print(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
