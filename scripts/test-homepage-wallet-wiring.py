from pathlib import Path


repo_root = Path(__file__).resolve().parents[1]
index_html = (repo_root / "site" / "index.html").read_text(encoding="utf-8")
autonomous_js = (repo_root / "site" / "autonomous.js").read_text(encoding="utf-8")

if index_html.count("data-connect-wallet") != 1:
    raise SystemExit("homepage must expose exactly one Connect Wallet control")

if index_html.count("data-wallet-provider") != 1:
    raise SystemExit(
        "homepage Connect Wallet requires exactly one provider selector for the shared wallet controller"
    )

provider_markup = index_html[index_html.index("data-wallet-provider") - 80 : index_html.index("data-wallet-provider") + 160]
if "hidden" not in provider_markup:
    raise SystemExit("homepage provider selector must remain hidden from the streamlined navigation")

for required in ("discoverProviders()", "selectProvider(context)", 'walletRequest("eth_requestAccounts")'):
    if required not in autonomous_js:
        raise SystemExit(f"shared wallet controller missing required wiring: {required}")

print("homepage wallet wiring is valid")
