#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

cd "${ROOT_DIR}"
mkdir -p "${ROOT_DIR}/screenshots"

echo "======================================================================"
echo "Pipewire Control: Generating Visual Verification Screenshots"
echo "======================================================================"

# Run the screenshot generation harness in headless mode
cargo run -- --generate-screenshots

echo "======================================================================"
echo "Visual verification screenshots successfully generated in:"
echo "  ${ROOT_DIR}/screenshots/"
echo "======================================================================"
ls -lh "${ROOT_DIR}/screenshots/"
