# ADR-0004: Themes are plain CSS packages, and the five designs are themes

- Status: accepted
- Date: 2026-11-01

## Context

The listener wants to restyle lull freely, and to let other people write themes "without much complexity but full freedom" (PROJECT.md). Today each design is a large stylesheet scoped by `:root[data-design=…]` plus a `design.js` that replaces six pieces of markup (`brand`, `nowHero`, `emptyArt`, `coverArt`, `homeLead`, `miniPlayer`). There's no `@layer`, so overrides depend on load order.

## Decision

**A theme package** is a folder, or a `.lulltheme` zip of one:

| File | Required | What |
|---|---|---|
| `theme.json` | yes | `id`, `name`, `author`, `api` (theme-API version, starts at 1), `extends` (a built-in design or none), `shell` (`rail`, `topnav` or `dock`), `swatches`, `modes` (`day`, `night` or both) |
| `theme.css` | yes | Any CSS |
| `slots/<slot>.html` | no | Static markup for a slot, with `{{field}}` placeholders, HTML-escaped |
| `assets/` | no | Fonts and images, referenced relatively from `theme.css` |

**Cascade layers:**
- `@layer reset, tokens, base, components, screens, design, user`.
- Built-in CSS goes in its layer; a design's CSS in `design`; a listener's theme in `user`.
- A theme author never needs `!important` or a long selector to win.

**Two levels:**
- **Tokens:** the custom properties in `web/tokens.css` (colour, type, space, radius, stroke, shadow stacks, motion, z-index, sizes, plus `--blur`). A theme that sets a few tokens is complete.
- **Full CSS:** any rule, aimed at the documented stable hooks:
  - BEM classes;
  - `data-region`, `data-screen`, `data-act`, `data-nav`;
  - `data-theme` (day or night), `data-shell`, `data-motion`.
- `docs/themes.md` lists the hooks and slot fields. Renaming or removing one is a breaking change that raises `api`.

**The five designs become packages** in `web/designs/<name>/` with this exact layout. Their slot functions become slot templates. Any behaviour a slot needs moves into the shared component, behind a manifest option. Being written in this format is what proves it has the power the designs need.

**Safety:**
- **Themes never run code.** Slot templates are parsed into a `<template>` and sanitized with a pinned, vendored DOMPurify, with no scripts, event handlers or `javascript:` URLs.
- **The CSP limits fonts, images and styles to the hub** (`'self'`, `data:`, `blob:`), so a shared theme can't load from or report to anywhere else.
- **`?theme=safe`** always boots the default design.

**Where themes live:**
- The listener's themes are stored on the hub, under `<data_dir>/themes/<id>/` (`crates/themes`), so every device has them.
- Which theme a device uses stays per device, as in v1.
- Editing a file in that folder reloads open devices live (`theme.changed` event).

**Theme Studio,** in Settings › Appearance:
- start from any design;
- edit tokens with pickers and sliders, with every screen previewing live in day and night;
- edit the full CSS;
- see contrast warnings, and loops that name no rate, with an Add it for the rate (an imported theme is offered the same);
- import by drag and drop, and export.

## Alternatives

- **A theme DSL or JSON-only themes.** Easier to validate, but "full freedom" means CSS.
- **Shadow DOM and `::part`.** It's a big rewrite of a buildless UI, for isolation lull doesn't need.
- **Themes with JavaScript.** It's what the built-ins do today, but a shared theme could then read the listener's data.

## Consequences

- **The conversion must be invisible.** The five designs have to look pixel-identical afterwards; the Quality session's visual matrix checks this.
- **Hooks are a public API.** UI refactors keep them stable or bump `api`.
- **The slot template language is deliberately tiny:** fields only, no logic.
