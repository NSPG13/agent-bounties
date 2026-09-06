"""Write only the public WalletConnect project identifier into the static site."""
import json
import os
import re
from pathlib import Path

project = os.environ.get("WALLETCONNECT_PROJECT_ID", "")
if not re.fullmatch(r"[a-fA-F0-9]{32}", project):
    raise SystemExit("WALLETCONNECT_PROJECT_ID must be a public 32-character hex project ID")
path = Path(__file__).resolve().parents[1] / "site" / "phone-wallet-config.js"
path.write_text(
    "// Public relay project identifier, populated by the Pages deployment.\n"
    f"window.agentBountiesPhoneWalletConfig = Object.freeze({{ projectId: {json.dumps(project)} }});\n",
    encoding="utf-8",
)
