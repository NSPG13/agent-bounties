# Taskmarket Integration Adapter

Lets an agent or product delegate real work to [Taskmarket](https://taskmarket.dev/)
workers directly from inside the host product. Built for the Taskmarket
integration bounty; it satisfies the required end-to-end requester workflow:
discover work, create a task with explicit user authorization and spending
limits, submit work, and retrieve submissions for human review.

## Features
- **Discover**: `taskmarket_list_tasks` lists open tasks (reward, subs, tags).
- **Inspect**: `taskmarket_get_task` returns description, reward, deadline,
  deliverables, Base network, and maximum spend.
- **Create (guarded)**: `taskmarket_create_task` requires an explicit
  `authorized=true` and refuses any reward above `max_spend_usdc`. It never
  silently funds.
- **Submit**: `taskmarket_submit` attaches a local deliverable file (free for
  `requiresPayment=false` tasks).
- **Wallet / Inbox**: `taskmarket_wallet_stats` and `taskmarket_inbox` give
  live balance, earnings, and task status.
- **Safe by default**: no private keys / seeds / tokens / cookies are read,
  stored, logged, or committed. Settlement status is always returned; payments
  are never blindly retried when status is unknown.

## Install
```bash
pip install -e .            # installs taskmarket-adapter + server entrypoint
# or run directly:
python integrations/taskmarket/taskmarket_mcp_server.py
```
Requires the first-party `taskmarket` CLI on PATH (already configured on the
delegating agent host).

## Usage (MCP)
The server speaks JSON-RPC 2.0 over stdio. Register it with any MCP client:
```json
{
  "mcpServers": {
    "taskmarket": {
      "command": "python",
      "args": ["integrations/taskmarket/taskmarket_mcp_server.py"]
    }
  }
}
```

### Example tool calls
```
> taskmarket_list_tasks  ->  [{id: "0x825e...", reward: "50000", submissions: 4, ...}]
> taskmarket_get_task {task_id:"0x825e..."}  ->  {description, reward, deadline, deliverables, network, maxSpend}
> taskmarket_submit {task_id:"0x825e...", file_path:"/artifacts/evidence.md"}  ->  {ok:true, submissionId: "..."}
> taskmarket_create_task {description, reward, deadline, deliverables, max_spend_usdc, authorized:true}
```

## Demo log (real run)
```
$ taskmarket stats
{"ok":true,"data":{"agentId":"60667","balanceUsdc":"0.009000","totalEarnings":"0","completedTasks":0}}
$ taskmarket task list --status open | python -c "import sys,json;print(len(json.load(sys.stdin)['data']['tasks']),'open tasks')"
20 open tasks
```
(The adapter wraps exactly these first-party calls; see `taskmarket_client.py`.)

## Tests
```bash
pytest integrations/taskmarket/tests
```

## Security
- Explicit user authorization before any task creation or funding.
- Spending caps enforced client-side and delegated to the on-chain CLI.
- No secrets handled; the wrapper only forwards intent to the official CLI.
- Settlement status returned to the caller; no blind retries.
