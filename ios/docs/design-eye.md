# Design eye: how the iPhone app should look

Every lane follows this when it draws a screen. It is the short version of
why some screens feel right and others feel off when the numbers look fine.
The tokens live in `ios/Sources/StreamNook/Design/`; this file says when to
use which. Android's measurements are the floor, not the ceiling: where a
rule here looks better than Android, follow the rule.

## The five questions

Ask these of every screen before showing it, in this order.

1. **What anchors the eye?** One thing per region should win: the picture
   and title on a card, the name on a player header, the filled button on a
   page. If two things tie, demote one.
1. **What belongs together?** Things about the same subject sit closer to
   each other than to anything else. Measure the gaps, then check that the
   inner gap is at most half the outer gap.
1. **Does anything say the same fact twice?** A LIVE chip on every card of
   a live list, a category name on every card of that category's page, an
   `@login` that matches the display name. Say it once.
1. **Does any control out-pull the content?** Buttons, chips and icons serve
   the title beside them. They are lighter, smaller or quieter than it.
1. **Does anything look off-centre or misaligned?** Check left edges line up
   down the screen, and that asymmetric shapes are nudged (see Optical
   nudges). Look at a full-resolution crop, not the third-size shot.

## Spacing

The scale is 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 56 points. Nothing else.

| Gap | Use |
| --- | --- |
| 2 to 4 | Lines of one text block (title, name, category) |
| 6 to 8 | A label and its icon; a heading and its one-line description |
| 8 | A card's inset around its picture; chips from the picture edge (6) |
| 12 | Cards in a grid or list; rows in a panel |
| 16 | The screen gutter; a panel's inner padding |
| 24 to 32 | Between sections; between a text block and its button |

Grouping rules:

- A heading's description sits close to it (6 to 10), the action under them
  sits further (16 to 28). Equal gaps everywhere read as a list of unrelated
  things.
- Every screen shares one left edge at 16 points: the large title, the
  search field, segments, cards, section labels. A grid inset of 14 next to
  a field inset of 16 is visible as a 2 point step.
- Text inside a card starts 2 points in from the picture's edge, so the
  letters' side bearings land on the picture's line.
- No dead bands: a row of chrome that holds one small control (a segment
  with two choices, a lone toggle) should share a row with something else
  where it can.
- The bottom safe area is not a margin. A bar at the screen's foot (the
  chat composer, a toolbar) reaches into the home-indicator inset and keeps
  only the strip the indicator needs: its field ends about 20 points above
  the screen edge. Stopping at the safe-area line and then padding again
  left a 44 point black band under the composer.

## Type

Satoshi throughout (`Font.sn(size, weight)`; tokens in `Typography.swift`).

| Role | Size and weight | Color |
| --- | --- | --- |
| Tab title (navigation large) | 32, 700 | primary |
| Drill-in title (navigation inline) | 17, 700 | primary |
| Wizard title | 30, 600 (44 on welcome) | primary |
| Landing title | 24, 700 | primary |
| Sheet title | 16, 600 | primary |
| Button label | 16, 600 | per button kind |
| Row title | 15, 500 | primary |
| Row strong (account names) | 15, 600 | primary |
| Card title | 14, 600 | primary |
| Body | 14, 400 | primary or secondary |
| Card channel line | 13, 400 | secondary |
| Row detail, card meta | 12.5, 400 | muted |
| Caption | 12, 400 | muted |
| Section label | 12, 600, uppercase, 0.6 tracking | muted |
| Chip | 11, 600, 0.2 tracking | white over a veil |

Rules:

- Two neighbouring levels of one block differ in at least two of size,
  weight and color. 13 medium over 13 regular in a slightly different grey
  reads as one level; that was the old card title.
- Three color levels at most in one block: primary, secondary, muted.
- No sizes off the table. Retire 13.5, 11.5, 10 and 8 when you touch a file.
- Explanatory copy (an empty list, a sign-in prompt) is secondary, not
  muted: muted is for the line the eye should skip.

## Icons beside text

- An inline symbol takes the text's weight and is 1 to 2 points smaller
  than the text (13 point text, 11 to 12 point symbol). A thin symbol next
  to semibold text looks detached; a bold one next to regular text shouts.
- A tick or chevron is never heavier than the words it marks: label size,
  one weight up at most, accent or muted color.
- One bar of controls uses one size, one weight and one fill style (all
  outlines or all fills). The player bar is 20 point medium outlines.
- Controls in a toolbar or player bar are lighter than the title they
  serve. If a button competes with the title, drop its weight first, then
  its size.

## Optical nudges

Box-centred is not eye-centred. These are in `Optical.swift` and the views
named; reuse them rather than re-deriving.

- **Play triangle** (`play.fill` in a round or square button): move it right
  by 7 percent of the symbol's point size (2 points at 28). Its mass sits at
  the flat left side. Pause needs nothing.
- **Penrose mark**: a right-pointing triangle whose cubes mass at the
  upright left side, a tenth of the width left of the box's middle. Centred
  marks move right by 6 percent of their width (`PenroseMark.centred`, on by
  default). Turn it off only when the mark sits beside text on a left edge.
- **Chevrons** (disclosure, back): no horizontal nudge; they sit on the row
  title's centre line, not the centre of a two-line block.
- **Circles next to squares**: a round control looks smaller than a square
  of the same size; give it about 10 percent more.
- **Avatars beside a text column** align to the column's top line and are
  roughly as tall as the first two lines (32 points next to 14 over 13).

## Corners

| Element | Radius |
| --- | --- |
| Buttons, segments, pills (44 points or less) | Capsule |
| Chips over pictures | 8 (compact 5) |
| Rows, press highlights | 8 |
| Thumbnails inside a card | 10 |
| Panels, category tiles, the mini player | 12 |
| Stream cards | 18 |
| Toasts | 18 |
| Sheets, menus, popovers | the system's |

A corner inside another is the outer radius less the gap between them
(card 18 = thumbnail 10 + inset 8), so the two curves run parallel. When
the inset is as large as the outer radius, pick the inner radius from the
table instead.

## Buttons

`SNButtonStyle` in `Controls.swift`: every worded button is 50 points tall
and a capsule, so swapping one for another never moves what is around it.

- **Primary** (`.snPrimary`): accent fill. One per screen at most: the way
  on.
- **Brand** (`.snBrand(color)`): a platform's own color, for its sign-in
  only. "Sign in with Twitch" is always `SignInButton`, purple with the
  glyph, wherever it appears.
- **Secondary** (`.snSecondary`): glass. Page-level actions beside a
  primary, or the only action where it should not shout.
- Never `.buttonStyle(.glass)` with an inner `frame(height:)`: the system
  adds its own padding and the button comes out 60 points or more.
- A status is not a button. "Notifications enabled", "Connected", "Live"
  are a mark and words, never a disabled capsule.

## Text and artwork

Text never sits on top of artwork, thumbnails or box art (Brandon: names
drawn on box art are "hard to read on top of some of the category art").
Labels go in a chin below the picture or in a row beside it. The one
exception is the small status chips over a stream preview (DROPS, hype
train, viewers, streak): they sit on a frosted veil, never bare on the
picture.

## Chips and pills

- A chip may pull less than the title under its picture. Tints stay under
  20 percent over the veil; only live red is saturated, and only as a dot.
- In a list where every item is live, the live dot rides the viewers chip.
  No separate LIVE chip.
- In rows (the list view) state rides the text line as colored words
  (the watch streak in amber), not as a chip: a chip there is the heaviest
  thing in the column.
- Chat header objects keep the desktop's one scale: 26 point capsules,
  22 point pills, 11.5 point semibold text, 18 point faces.

## Glass and surfaces

- Our own surfaces use `snPanel`, `snMediaCard`, `snGlass`, `snChipVeil`;
  each goes opaque at Glassiness 0. System chrome (tab bar, sheets, menus)
  stays Liquid Glass.
- No frost inside frost; menus over video stay opaque enough to read.
- Media cards have no outline or top light: a grid of lit rectangles is
  noise.

## Before you show it: the polish pass

Every visual change gets a polish pass on real phone screenshots of every
state before it is called done (Brandon's standing rule; Brain
`references/Visual_Polish.md`). Run the `visual-polish` agent on the shots,
or work the list by hand.

1. **Shoot every state.** Drive the screen on the phone
   (`/root/sn-tools/drive.sh`) with a `wait:4` after each `shot`, so the
   next step does not land in the picture: loading, empty, loaded, one line
   and two lines, signed in and out, Glassiness 0, portrait and landscape.
1. **Alignment by eye.** Controls beside a heading sit on the heading's
   optical middle (baseline minus half the cap height), not its line box.
   Crop the full-resolution shot and measure, then judge by eye too.
1. **Hierarchy.** One loudest thing per region; accessory actions a step
   quieter than the information they sit with.
1. **Grouping.** Related lines close, unrelated groups clearly apart, gaps
   on the spacing scale.
1. **Edges and reveals.** Nothing appears or vanishes with a hard cut;
   overflowing strips fade at the hidden edge.
1. **Consistency across states.** Nothing jumps between states (a button
   changing height between wizard pages moved the step dots 14 points).
1. **Contrast.** Text and icons meet WCAG AA on the lightest and darkest
   backgrounds they can land on, video included.
1. **Words.** US spelling, no em dashes, plain words.
1. **Fresh eyes.** Ask the five questions at the top, fix, and look once
   more.

Related standing rules: `chat-header-scale`, `design-pass-before-presenting`,
`glassiness-zero-contract`, `no-ui-moves-down`, `one-place-then-offer-choices`,
`no-mini-player-previews` (Brandon's preferences in the StreamNook memory
folder), and `ios/docs/parity/shell-design.md`.
