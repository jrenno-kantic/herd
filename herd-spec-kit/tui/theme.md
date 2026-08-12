# Theme

Defined in `theme.rs`. Dark background throughout.

| Element | Style |
|---|---|
| background / normal | white on black |
| selection | black on green, bold |
| logs / secondary text | gray |
| command bar | yellow |
| borders | dark gray |
| status: ready | green, bold |
| status: starting | yellow, bold |
| status: error | red, bold |
| favourite `★` | gold (yellow), bold |
| unreferenced cache entry | cyan |

## Semantic reuse

The three status styles carry meaning beyond the status bar, and are used
consistently for it:

| Meaning | Style | Where |
|---|---|---|
| serving, healthy, a good result | `status_ready` | status bar, lifecycle glyph `●`, the stats line of a successful probe |
| in transition, or a caveat worth noticing | `status_starting` | STARTING / STOPPING, `◐`, a tight memory fit, an overridden setting, "working…" |
| failed, or beyond what the machine can do | `status_error` | ERROR, `✖`, a preset too large for the budget, the port-conflict modal, the reserved-memory caution |

A row is only ever drawn in `status_error` when the condition is *known*: a
preset whose size cannot be parsed, or a machine whose RAM cannot be read, stays
in the normal style rather than being flagged on a guess.

Two colours outside that scale, each for something that is not a severity:

- **Gold** marks a favourite — and the `★` glyph carries the meaning, not the
  colour, so it survives a screenshot, a colour-blind reader and a terminal with
  its own palette. It is not drawn in gold on the selected row, where gold on
  the selection's green is unreadable.
- **Cyan** marks a cached model no preset in this tier names. Deliberately not
  red: it is not an error — it may belong to another tier — and a third meaning
  on red or amber would read as a fourth severity.
