import argparse
import hashlib
import json
import re
import shutil
import xml.etree.ElementTree as ET
from pathlib import Path


DIAGRAMS = ("fixed-step-fracture.svg", "asset-pipeline.svg", "force-diagnostics.svg")


def main() -> None:
    parser = argparse.ArgumentParser(description="Package the architecture Pages site.")
    parser.add_argument("destination", type=Path)
    parser.add_argument("commit")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.commit):
        parser.error("commit must be a full Git SHA")

    docs = Path(__file__).resolve().parents[2] / "docs"
    index_source = docs / "architecture.html"
    index_bytes = index_source.read_bytes()
    for name in DIAGRAMS:
        if f'src="images/{name}"'.encode() not in index_bytes:
            raise ValueError(f"The architecture page must reference {name}")
        if ET.parse(docs / "images" / name).getroot().tag != "{http://www.w3.org/2000/svg}svg":
            raise ValueError(f"Expected an SVG document for {name}")

    images = args.destination / "images"
    images.mkdir(parents=True)
    for name in DIAGRAMS:
        shutil.copyfile(docs / "images" / name, images / name)
    (args.destination / "index.html").write_bytes(index_bytes)

    proof = {"commit": args.commit, "index_sha256": hashlib.sha256(index_bytes).hexdigest()}
    (args.destination / "bevy-cicd-build.json").write_bytes(
        (json.dumps(proof) + "\n").encode("utf-8")
    )


if __name__ == "__main__":
    main()
