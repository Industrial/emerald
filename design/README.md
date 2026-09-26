# Emerald visual identity

This directory holds the source files for Emerald's visual identity: the gem mark, the
"Emerald" wordmark (JetBrains Mono Bold, converted to static path outlines), and the
mark+wordmark lockups, banner, and avatar composed from them, all under `design/src/`.
Everything here is dedicated to the public domain under CC0 1.0 Universal — see
[`./LICENSE`](./LICENSE) — independent of whatever license covers the rest of this
repository. `design/tools/` (a separate, standalone Rust CLI) regenerates the platform
asset set (favicons, app icons, social-preview and avatar PNGs, etc.) from these SVG
sources.

## Regenerating the asset set

```bash
cargo run --manifest-path design/tools/Cargo.toml -- generate --input-dir design/src --out-dir design/dist
cargo run --manifest-path design/tools/Cargo.toml -- verify --dir design/dist
```

Output lands in `design/dist/`, which is gitignored — it's regenerable build output,
not something to commit. The one exception is `design/src/banner.svg`, which the root
`README.md` embeds directly as inline SVG (no raster export needed for that one).

## Manual uploads (not automated — do these by hand)

Two of the generated assets are one-time uploads through GitHub's settings UI, not
things this repo tracks or that any tool here performs automatically:

- **Social-preview card**: upload `design/dist/social-preview-1280x640.png` at
  **repo Settings → General → Social preview**.
- **Org/profile avatar**: upload `design/dist/avatar-512.png` (or `avatar-192.png`
  for a smaller source) at the organization's or repo's profile picture settings.

Re-run the generator and re-upload whenever `design/src/mark.svg`, `banner.svg`, or
`avatar.svg` change.
