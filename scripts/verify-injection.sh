#!/usr/bin/env bash
# whisrs verify-injection — MANUAL verification of the keyboard-injection
# backends (issue #44).
#
# This is a manual eyeballing aid: it types a fixed mixed-script string via
# the Wayland virtual-keyboard backend, but it CANNOT assert what actually
# landed in your window. Focus a text field (editor, browser input, etc.)
# within the countdown, then check that every character below appears
# correctly — including the Arabic, Greek, Cyrillic, and math symbols, which
# the layout-dependent uinput backend would garble on a Latin-only layout.
#
# Expected output in the focused field:
#
#     ammeter اميتر ω д 1+2=3
#
# Usage:
#   ./scripts/verify-injection.sh            # default: wayland-vk backend
#   BACKEND=auto ./scripts/verify-injection.sh
#   BACKEND=uinput ./scripts/verify-injection.sh   # for comparison
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BACKEND="${BACKEND:-wayland-vk}"
DELAY_MS="${DELAY_MS:-4}"
TEXT='ammeter اميتر ω д 1+2=3'

echo "whisrs injection verification (manual)"
echo "--------------------------------------"
echo "backend: ${BACKEND}"
echo "string : ${TEXT}"
echo
echo "Focus a text field within the next 3 seconds."
echo "Then check that the typed text matches the string above exactly."
echo
for n in 3 2 1; do
    echo "  starting in ${n}..."
    sleep 1
done

cargo run --quiet \
    --manifest-path "${REPO_ROOT}/Cargo.toml" \
    -p xkb-type \
    --features wayland-vk \
    --example type -- \
    --backend "${BACKEND}" \
    --delay-ms "${DELAY_MS}" \
    "${TEXT}"

echo
echo "Done. Eyeball the focused field — every glyph should match:"
echo "    ${TEXT}"
