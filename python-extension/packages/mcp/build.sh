#!/usr/bin/env bash
# mcp 2.3, the Model Context Protocol's Python SDK (clients and servers: stdio, SSE, streamable
# HTTP; FastMCP), with what it needs that the Python image has not: pure Python, installed from
# their wheels as they are, several distributions in one tree (each wheel's RECORD says which
# files are its; tools/install-tree.py writes the RECORD and INSTALLER of each). What it needs
# that is compiled is its own package here: rpds-py (jsonschema's and referencing's), cryptography
# (pyjwt[crypto]'s), so those come first in PACKAGES; pydantic, anyio, httpx2, h11 and the rest
# are the Python image's (python/packages.lock).
# The `cli` and `rich` extras (typer, rich, python-dotenv) are not in: `pip install mcp` leaves
# them out too.
# Run by ../../build.sh (packages/README.md); result: build/mcp/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
SITE="$BP/site"
mkdir -p "$BP"

# mcp's requirements resolved for CPython 3.13 on Linux (requires_dist, no extras but pyjwt's
# crypto), less what the image and the other packages have
WHEELS=(mcp mcp-types jsonschema jsonschema-specifications referencing attrs opentelemetry-api pyjwt
        python-multipart sse-starlette starlette uvicorn click)

if stage mcp "$SITE/.done" $(lock "${WHEELS[@]}") "$PYEXT/packages/mcp/build.sh"; then
  rm -rf "$SITE"; mkdir -p "$SITE"
  for wheel in "${WHEELS[@]}"; do
    "$HOSTPY" -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$(fetch "$wheel")" "$SITE"
  done
  n="$(find "$SITE" -maxdepth 1 -name '*.dist-info' | wc -l)"
  [[ $n == "${#WHEELS[@]}" ]] || { echo "$n .dist-info for ${#WHEELS[@]} wheels" >&2; exit 1; }
  touch "$SITE/.done"
fi
