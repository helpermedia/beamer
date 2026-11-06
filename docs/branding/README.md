# Beamer branding

The Beamer logo, icons and banner.

| File | Use |
|---|---|
| `beamer.svg` | The logo on light backgrounds |
| `beamer-on-dark.svg` | The logo on dark backgrounds |
| `beamer-icon.svg` | The A on its own, as a symbol |
| `beamer-mono.svg`, `beamer-icon-mono.svg` | One colour, for places that set their own colours, such as plugin interfaces, badges and print. Inline in a page they take the colour of the surrounding text; as an image they show black. |
| `favicon.svg`, `favicon.ico` | Browser favicon: the A redrawn on a 16-pixel grid, so it stays sharp at 16, 32 and 48 px |
| `apple-touch-icon.png` | Home screen icon, 180 px |
| `beamer-banner.png` | Banner at the top of the repository's README: the logo with a spot of light under the A and the tagline, 3200 x 640 px for sharp screens |

## Colours

| Colour | Hex | CSS |
|---|---|---|
| Blue | `#206fff` | `oklch(58.5% 0.227 261.3)` |
| Ink | `#101010` | `oklch(17.5% 0 0)` |
| Paper | `#f4f2ee` | |

Ink is the letter colour on light backgrounds and the ground of the banner and the home screen icon; paper is the letter colour on dark ones. The blue works on both.

## Favicon

In a page's head:

```html
<link rel="icon" href="/favicon.ico" sizes="32x32">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
```

The `sizes` attribute on the ICO line keeps Chrome on the SVG.
