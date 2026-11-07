# ADR-0083: The UI gates check what works, never the shape

- Status: accepted (the owner approved it on 2026-11-07)
- Date: 2026-11-07

## Context

Each design is a world of its own (ADR-0004), and a theme may restyle anything through the hooks in docs/themes.md.
The UI gates (docs/context/quality.md#ui-gates) hold every built-in design to what lull must do at every width, and
some of them had come to hold it to lull's own shape as well. The chrome check behind the fit gate (`tests/ui/checks.mjs`
`chrome`) failed a control whose box reached outside its bar ("sticks out"), any two controls whose boxes met ("on"),
and a box inside one control that met another; Now Playing's gate failed any two panels whose boxes met; the side
panel's strip failed a meta that took two lines or stood outside the panel. Each of those was written against a real
defect, but each also fails a deliberate shape: a player bar that is a capsule narrower than the window, a play key at
the centre of the song's disc, a seek drawn as the capsule's tube, a title over its art.

That held the designs back. A new default design for lull had to pull parts back into lull's rectangle in round after
round, not because anything stopped working but because the gate read a choice of shape as a bug. The owner asked
for gates that guard what lull does and leave its look free.

## Decision

The gates check function, never form.

**Checked (function), at every width from 360 to 2560 px:**
- every control is there and can be clicked: on the screen, and reached by a click at some point of it (a hit test
  over its box, its centre first, then a 5 × 5 grid: a control covered at every point fails, and the failure names what
  covers it);
- each control's hit target is large enough (axe's `target-size`, WCAG 2.2 AA, in the accessibility gate);
- no text is under an element that paints over it (a background or a picture; an empty box stacked over a line hides
  nothing), drawn on other text (the lines' letters, not their line boxes, which overlap in tight headings), off the
  screen, or cut without an ellipsis;
- a page is judged once it has arrived (every animation that ends has ended), so an entrance that slides it in passes,
  and content that scrolls under a part laid over its scroller from outside it (the page under the chrome, lyrics
  under a foot) passes, since it scrolls clear;
- the essentials are there (play, previous, next, the player's "…", search);
- keyboard and screen-reader order are lull's markup order, which a design's CSS doesn't change (the accessibility
  gate checks what can be checked of them: names, focusable hidden parts).

**Not checked (form):** where a part sits; whether it rises out of its bar; whether a bar is full width or rectangular;
whether one control sits inside another's area; whether a region keeps its usual shape.

**Where a check can't tell a deliberate shape from a bug**, the design frees it in its own CSS, with the reason:
`--gate-free: "<check>: <why>"` on the part or a box round it (a custom property, so it inherits, and works for a
listener's theme as for a built-in one, with no change to theme.json, which the hub reads strictly). A check named
without a reason still runs and says so. The checks that can be freed: `even-tags` (tags side by side one height on
one centre line), `song-side-room` (the song side keeps its chips at 1440 px rather than giving them to the "…" menu),
`seg-indicator` (a segmented control's indicator lies on the picked choice). docs/themes.md section 5 lists them.

The performance budgets, the 1 KB shipped-bytes rule and the goldens are unchanged: they measure cost and catch
unintended change, not shape.

## Alternatives

- **Keep the box checks and let a design list exceptions** (a gate allowlist per design). Every new shape would need
  an entry before it could land, which is the friction this removes, and the list would grow with every design.
- **Loosen the thresholds** (allow a few pixels out of the bar). Still fails a capsule or a key in a disc, and lets a
  real defect of a few pixels through.
- **Opt-outs in theme.json.** The hub rejects unknown manifest fields, so this needs a hub change and a manifest
  version; the reason would sit away from the CSS it explains. A custom property sits on the part it frees.

## Consequences

- A design may shape and place its chrome as it likes; the gates fail it only when a control can't be clicked or text
  can't be read.
- A hit test is stricter than a box check in one way: a decorative layer that catches clicks over a control now fails,
  where box checks ignored it. It found two on main as this landed: Glasshouse's cover glow, drawn 1.45 times the art's
  size, took every tap on a phone's "…" and "playing from" above it; lull's lyrics window, given less room than its
  lyrics' minimum when the source details opened, ran its lines and its text-size keys under the source window. Both
  are fixed with it.
- Text drawn over a control's artwork (a label on a disc) passes; text over text fails, as unreadable.
- A defect the box checks caught only through shape (a part pushed out of a bar by a layout bug, but still clickable
  and readable) now passes the fit gate; the goldens still show it.
