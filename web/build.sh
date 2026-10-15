#!/usr/bin/env bash
# Builds the WebGPU demos into the repository's dist/ directory. Run from any directory.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$ROOT"

# On Windows `python3` can be the Microsoft Store stub, which exists but doesn't run.
PYTHON=python3
if ! python3 -c '' >/dev/null 2>&1; then
    PYTHON=python
fi

# Keep this query usable before wasm-bindgen-cli is installed (including in CI).
BINDGEN_VERSION="$("$PYTHON" - <<'PY'
import pathlib
import tomllib

lock = tomllib.loads(pathlib.Path('Cargo.lock').read_text(encoding='utf-8'))
versions = sorted({p['version'] for p in lock['package'] if p['name'] == 'wasm-bindgen'})
if len(versions) != 1:
    raise SystemExit(f'Expected one wasm-bindgen version in Cargo.lock; found {versions}')
print(versions[0])
PY
)"
if [[ "${1:-}" == "--bindgen-version" && $# == 1 ]]; then
    printf '%s\n' "$BINDGEN_VERSION"
    exit 0
fi
if (( $# != 0 )); then
    printf 'Usage: bash web/build.sh [--bindgen-version]\n' >&2
    exit 2
fi

for tool in cargo wasm-bindgen; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'Missing required tool: %s\n' "$tool" >&2
        printf 'Install the CLI with: cargo install wasm-bindgen-cli --version %s --locked\n' "$BINDGEN_VERSION" >&2
        exit 1
    fi
done
ACTUAL_VERSION="$(wasm-bindgen --version)"
if [[ "$ACTUAL_VERSION" != "wasm-bindgen $BINDGEN_VERSION" ]]; then
    printf 'wasm-bindgen mismatch: lock requires %s, installed CLI reports %s\n' "$BINDGEN_VERSION" "$ACTUAL_VERSION" >&2
    printf 'Run: cargo install wasm-bindgen-cli --version %s --locked --force\n' "$BINDGEN_VERSION" >&2
    exit 1
fi

# Respect Cargo's configured target directory rather than assuming target/.
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version 1 | "$PYTHON" -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
DEMOS=(lighting_test realtime_shadows inspector stress_lights)
OPT_FLAGS=(--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext
    --enable-reference-types --enable-multivalue --enable-simd --enable-mutable-globals)
if command -v wasm-opt >/dev/null 2>&1; then
    # New LLVM releases can emit bulk-memory-opt instructions. Older Binaryen
    # releases do not have this switch, so only pass it when supported.
    OPT_HELP="$(wasm-opt --help)"
    if [[ "$OPT_HELP" == *--enable-bulk-memory-opt* ]]; then
        OPT_FLAGS+=(--enable-bulk-memory-opt)
    fi
else
    printf 'wasm-opt not found; publishing unoptimized wasm-bindgen output.\n' >&2
fi

mkdir -p dist
cp web/index.html web/style.css dist/
for demo in "${DEMOS[@]}"; do
    printf '\nBuilding %s for WebGPU…\n' "$demo"
    FEATURES=()
    if [[ "$demo" == inspector ]]; then
        FEATURES=(--features inspector)
    fi
    cargo build --locked --profile wasm-release --target wasm32-unknown-unknown \
        --example "$demo" "${FEATURES[@]}"
    mkdir -p "dist/$demo"
    wasm-bindgen --target web --no-typescript --out-dir "dist/$demo" \
        --out-name "$demo" "$TARGET_DIR/wasm32-unknown-unknown/wasm-release/examples/$demo.wasm"
    if command -v wasm-opt >/dev/null 2>&1; then
        wasm-opt -Oz "${OPT_FLAGS[@]}" "dist/$demo/${demo}_bg.wasm" \
            -o "dist/$demo/${demo}_bg.opt.wasm"
        mv "dist/$demo/${demo}_bg.opt.wasm" "dist/$demo/${demo}_bg.wasm"
    fi
    # Bevy resolves its default asset root relative to each demo document. Browser demos bake
    # in memory, so local `.dynlight` bakes stay out of the site.
    if [[ -d assets ]]; then
        mkdir -p "dist/$demo/assets"
        (cd assets && find . -type f ! -name '*.dynlight' \
            -exec cp --parents {} "$ROOT/dist/$demo/assets/" \;)
    fi
done

"$PYTHON" - <<'PY'
import pathlib
import shutil

root = pathlib.Path('.')
# Title, description, controls and player-bar metrics for each demo page.
demos = {
    'lighting_test': (
        'Lighting laboratory',
        'Every light type and effect, metallic surfaces and volumetric fog, baked in your browser. '
        'Drive a cube through it with a lantern that casts realtime shadows.',
        '<span><kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd> or arrow keys Move the cube '
        '(the same keys on Colemak: W A R S)</span>'
        '<span><kbd>Shift</kbd> Run</span><span><kbd>Space</kbd> Lantern on or off</span>'
        '<span>Drag to orbit, scroll to zoom</span>'
        '<span><kbd>B</kbd> Rebake</span><span><kbd>V</kbd> Cycle debug views</span>'
        '<span>Click the canvas first. The camera drifts on its own when left alone.</span>'
        '<span>Add <code>?debug-view=light-count</code> to the address to start in a debug view.</span>',
        ''),
    'realtime_shadows': (
        'Shadows in motion',
        'Moving lights and geometry with realtime variance shadow cubemaps. Nothing is baked.',
        '<span>The lamp and crates move on their own.</span>',
        ''),
    'inspector': (
        'Light inspector',
        'Edit one light live with the Feathers inspector, then bake its shadows in the browser.',
        '<span>Drag the sliders to change hue, intensity, radius, cone angle and falloff.</span>'
        '<span>Cycle the type and effect, toggle realtime shadows, and press '
        '<strong>Bake (preview)</strong>.</span>',
        ''),
    'stress_lights': (
        'Many lights, one scene',
        '256 moving realtime lights among the pillars of an 80 m hall, nothing baked. '
        'Compare the frame time on your hardware.',
        '<span>Drag to orbit, scroll to zoom. The camera drifts on its own when left alone.</span>'
        '<span>Add <code>?lights=1024</code> to the address for more lights.</span>'
        '<span>Browsers cap the frame rate at the display refresh rate.</span>',
        '<p class="metric"><span>Frame time</span> '
        '<output id="frame-time">Waiting for engine…</output></p>'),
}
template = (root / 'web/demo.html').read_text(encoding='utf-8')
thumbs = root / 'dist/thumbnails'
thumbs.mkdir(parents=True, exist_ok=True)

for name, (title, description, controls, metrics) in demos.items():
    page = template
    for key, value in {'DEMO': name, 'TITLE': title, 'DESCRIPTION': description,
                       'CONTROLS': controls, 'METRICS': metrics}.items():
        page = page.replace('__' + key + '__', value)
    (root / 'dist' / name / 'index.html').write_text(page, encoding='utf-8')
    # Native screenshots, made with each example's `--frames N --screenshot` flags.
    shutil.copyfile(root / 'web/thumbnails' / f'{name}.png', thumbs / f'{name}.png')
PY

printf '\nSite ready in %s/dist\nServe over HTTP locally, for example: python3 -m http.server 8000 --directory dist\n' "$ROOT"
