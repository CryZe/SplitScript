# SplitScript icon

The project icon is the selected stopwatch with white code braces and three
colored ring segments. The gaps have parallel sides and a constant width.
The dial is 12.5% larger than the initial stopwatch design; braces sit
slightly farther apart, with clearance above the button and below the dial
to keep the stopwatch visually contained. These proportions are shared by
the README, extension listing, and sidebar. The dial remains circular, and its
uniform scale preserves the parallel ring gaps.

- `icon.svg`: canonical, transparent color artwork (256 × 256 viewBox).
- `icon-monochrome.svg`: the same geometry using `currentColor` for every fill
  and stroke. Inline it in HTML to inherit the surrounding CSS `color`.
- `icon-currentcolor.svg`: transparent color artwork with only the braces and
  hands using `currentColor`. Inline it to match surrounding text while retaining
  the red, green, and blue ring.
- `icon-light.svg`: transparent color artwork with dark braces and hands for
  light README themes. Dark themes use the original `icon.svg`.
- `../editors/vscode/media/icon.png`: an opaque 128 × 128 PNG for the extension
  listing.
- `../editors/vscode/media/icon-readme-light.png` and `icon-readme-dark.png`:
  transparent 128 × 128 PNG variants for the extension README.
- `../editors/vscode/media/splitscript-debugger.svg`: monochrome artwork at a
  nominal 24 × 24 size for the debugger Activity Bar container. A tighter
  `18 18 220 220` viewBox removes outer padding, enlarging the artwork by about
  16% while keeping every stroke inside the icon. VS Code uses its silhouette
  as a mask and supplies the theme color.

An SVG loaded through `<img>` does not inherit its parent document's `color`;
`currentColor` defaults to black in that case. The monochrome asset has no fixed
color or background. The color SVG retains its white braces and hands.
Both README headings use a `<picture>` element to choose the appropriate
transparent artwork for light and dark themes, following
[GitHub's supported theme-image mechanism](https://github.blog/changelog/2022-08-15-specify-theme-context-for-images-in-markdown-ga/).
This approximates text-color matching for README images; exact CSS color
inheritance is available through the inline `icon-currentcolor.svg`.

## Updating the artwork

Edit `icon.svg`, then regenerate the derived assets from the repository root:

```console
npm install --prefix target/icon-tools --no-save --package-lock=false sharp
node scripts/generate-icons.mjs
```

The optional rasterizer stays in the ignored `target` directory. Generated assets
are checked in, so normal extension builds do not need it.

VS Code requires a [raster extension icon of at least 128 × 128 pixels](https://code.visualstudio.com/api/references/extension-manifest).
The Marketplace [does not accept SVG listing icons or custom SVG README images](https://code.visualstudio.com/api/working-with-extensions/publishing-extension).
The extension README therefore uses transparent PNGs, while GitHub's repository
README uses transparent SVGs. The listing PNG's dark background is a contrast
choice, not a transparency requirement of PNG.
