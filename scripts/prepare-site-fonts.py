"""Acquire the unmodified, self-hosted Satoshi webfont from its official source.

The FFL permits use on our site but not redistribution as a repository asset.
Pin the exact font bytes; a changed release needs an explicit license review.
"""
from hashlib import sha256
from io import BytesIO
from pathlib import Path
from urllib.request import urlopen
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "site/assets/fonts/Satoshi-Variable.woff2"
DIGEST = "e739aff9b4d02c264341d6d4872edcda28e79373aeda936f659566a1cd3eb47f"


def main():
    if FONT.exists() and sha256(FONT.read_bytes()).hexdigest() == DIGEST:
        print("Self-hosted Satoshi font verified.")
        return
    with urlopen("https://api.fontshare.com/v2/fonts/download/satoshi", timeout=30) as response:
        archive = response.read(8_000_001)
    if len(archive) > 8_000_000:
        raise SystemExit("Font archive exceeds the expected size.")
    with ZipFile(BytesIO(archive)) as package:
        data = package.read("Satoshi_Complete/Fonts/WEB/fonts/Satoshi-Variable.woff2")
    if sha256(data).hexdigest() != DIGEST:
        raise SystemExit("Fontshare changed the font. Review the new version before updating the pin.")
    FONT.parent.mkdir(parents=True, exist_ok=True)
    FONT.write_bytes(data)
    print("Self-hosted Satoshi font prepared.")


if __name__ == "__main__":
    main()
