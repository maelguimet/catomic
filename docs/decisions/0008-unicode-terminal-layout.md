# Decision 0008: Unicode terminal layout helpers

Catomic uses `unicode-segmentation` for extended grapheme boundaries and
`unicode-width` for terminal cell widths.

The standard library exposes Unicode scalar iteration but does not implement
Unicode grapheme segmentation or terminal display width. Reimplementing those
evolving standards locally would be both larger and less correct.

Both dependencies are pure, `no_std`-capable text tables. Plain mode uses them
only while moving or rendering visible text; they construct no service and do
no startup, filesystem, process, or network work. Focused tests cover combining
marks, wide characters, emoji, clipping, and tab expansion. They can be removed
by reverting to scalar movement and one-cell-per-scalar rendering, with the
known Unicode display defects that motivated this decision.
