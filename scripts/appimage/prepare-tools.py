"""Preseed Tauri's tools cache with hash-verified assets, including mutable tags."""
import hashlib
import json
import os
from pathlib import Path
import urllib.request


def main():
    manifest = json.loads(Path(__file__).with_name("toolchain.json").read_text())
    cache = Path(os.environ.get("XDG_CACHE_HOME", str(Path.home() / ".cache"))) / "tauri"
    cache.mkdir(parents=True, exist_ok=True)
    for asset in manifest["architectures"][os.environ["CANDIDATE_ARCH"]]:
        with urllib.request.urlopen(asset["url"], timeout=120) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != asset["sha256"]:
            raise RuntimeError(f"Upstream asset changed: {asset['name']}; review toolchain.json before rebuilding")
        destination = cache / asset["name"]
        destination.write_bytes(data)
        destination.chmod(0o755)
        print(f"Verified {asset['name']}: {asset['sha256']}")


if __name__ == "__main__":
    main()
