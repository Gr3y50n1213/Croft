## Motion system

Motion in Croft uses springs, not fixed-time curves, so a gesture that is interrupted or reversed carries its velocity into the next movement. This follows the Material 3 Expressive motion scheme: **spatial** springs move or reshape things and may overshoot a little; **effects** springs change colour or opacity and never overshoot.

### Spring tokens

Values are the ones used by the Material 3 Expressive motion scheme (stiffness and damping ratio). The settle time is my estimate from those numbers for a spring that has come within 2 percent of its target; confirm on a real device. SwiftUI values come from response = 2π / √stiffness.

| Token | Stiffness | Damping | Settles in about | SwiftUI response | Use |
| --- | --- | --- | --- | --- | --- |
| Spatial fast | 800 | 0.6 | 0.24 s | 0.22 | Small things: switch thumbs, chips, swipe reveal, star pop |
| Spatial default | 380 | 0.8 | 0.26 s | 0.32 | Most movement: row collapse, sheets, container transforms |
| Spatial slow | 200 | 0.8 | 0.35 s | 0.44 | Full-screen moves and large shape changes |
| Effects fast | 3,800 | 1.0 | 0.07 s | 0.10 | Press feedback, state layers |
| Effects default | 1,600 | 1.0 | 0.10 s | 0.16 | Fades, colour changes, icon cross-fades |
| Effects slow | 800 | 1.0 | 0.14 s | 0.22 | Scrim, large colour shifts such as theme changes |
| Standard spatial fast, default, slow | 1,400, 700, 300 | 0.9 | 0.12, 0.17, 0.26 s | 0.17, 0.24, 0.36 | Used in place of the expressive set when Animation level is Reduced |

In Compose these come from `MaterialTheme(motionScheme = MotionScheme.expressive())`, so component animations pick them up automatically; custom animations ask the scheme for `fastSpatialSpec()`, `defaultSpatialSpec()`, `slowSpatialSpec()` and the three effects specs. The tokens also live in `design/tokens.json`, and the web and iOS layers generate their own code from it (CSS `linear()` easing approximating each spring on the web, `.spring(response:dampingFraction:)` on iOS), so the three apps feel alike.

### Rules

1. Move only `transform` and `opacity` (and colour) during an animation. Never animate layout or size in a way that makes text reflow; collapses clip instead.
2. Every animation is **interruptible**: touching the screen mid-animation hands control to the finger.
3. Nothing waits for an animation to finish before accepting input.
4. The first frame of feedback appears within one frame of the touch.
5. A screen that is not visible runs no animations.
6. Stagger is used rarely, at most 30 ms between items, and never on lists longer than 8 items.

### Animation catalogue

| Animation | Trigger | What moves | Token | Reduced-motion substitute |
| --- | --- | --- | --- | --- |
| Row to conversation | Tap a row | Row expands to the full conversation (container transform) | Spatial default | Cross-fade, 100 ms |
| Predictive back | Back gesture | Conversation shrinks and fades, list shows beneath | Spatial default, follows the finger | Plain fade |
| FAB to compose | Tap FAB | FAB grows into the compose sheet | Spatial default | Cross-fade |
| FAB extend and collapse | Scroll direction | Label slides and fades, FAB narrows | Spatial fast | Instant swap |
| Search pill to search | Tap | Pill expands to full screen | Spatial default | Cross-fade |
| Tab switch | Bottom bar tap | Fade through between destinations; indicator pill morphs | Effects default, spatial fast for pill | Instant swap |
| Settings drill-down | Open or close page | Shared axis X | Spatial default | Cross-fade |
| Sheet open and close | Tap, drag | Rises from the bottom; follows the finger when dragged | Spatial default | Fade |
| Dialog | Open | Scale from 90 percent, fade | Effects default | Fade |
| Menu open and close | Long-press, overflow tap, right-click | Menu scales from 90 percent at the pressed element and fades; closes with a fade | Effects default | Fade |
| Avatar to check | Select | Avatar flips to the check mark | Spatial fast | Cross-fade |
| Star toggle | Tap | Scale 1, 1.35, 1 with colour fill | Spatial fast | Colour change only |
| Long-press lift | Press held past the long-press time | The pressed element lifts 1 dp and grows to 102 percent; the menu or selection opens at that moment | Spatial fast | Highlight only, no scale |
| Unread change | Mark read or unread | Dot fades and shrinks; text weight cross-fades | Effects default | Same |
| Swipe reveal | Drag | Card offset, the pill that fills the gap, icon scale 1.2 at the arm point | Spatial fast | Colour and icon change only |
| Swipe commit | Release armed | Card slides off while the pill widens to full width, then both collapse | Spatial fast, then spatial default | 100 ms fade, short collapse |
| Swipe park or cancel | Release armed on Snooze or Move, or release short of the arm point | Armed: the card slides to 40 percent and holds while the sheet is open, then leaves or returns. Not armed: the card springs back and the pill shrinks into the gap | Spatial default for parking, spatial fast for cancel | Card moves without bounce; pill fades |
| Undo | Tap Undo | Row returns, brief highlight | Spatial default, effects default | Fade in |
| New mail | Arrival | Rows slide down (at top) or N new pill rises (scrolled) | Spatial default | Appear without movement |
| Pull to refresh | Drag | Expressive loading indicator grows, morphs at threshold | Spatial default | Static spinner |
| Send | Tap Send | Compose card shrinks toward the bottom, then the snackbar rises | Spatial default | Fade out |
| Button loading | Tap | Label to spinner, width fixed; success check for 600 ms | Effects default | Same |
| Change refused by the server | A permanent refusal after an optimistic change | The row returns to its place with a red edge that fades after 2 s, and the Retry snackbar rises | Spatial default, effects default | Row reappears; red edge without fade |
| Snackbar | Show and hide | Rises and fades | Spatial fast, effects default | Fade |
| Chip selection | Tap | Fill changes, check slides in | Spatial fast for width, effects for fill | Instant |
| Drag and drop | Drag a recipient chip, event or reorderable item | The item lifts with a shadow, neighbours make room, and it settles into the slot on drop | Spatial fast while dragging, spatial default on drop | Item jumps to the slot; no gap animation |
| Switch | Toggle | Thumb moves and grows, track colour changes | Spatial fast | Instant |
| Theme change | Pick a palette or mode | Whole app cross-fades colour roles | Effects slow | Instant swap |
| Skeleton loading | First load | Slow shimmer, 1.2 s loop | Linear | Static placeholders |
| Onboarding probes | Each probe resolves | Check pops in; row text fades to its result | Spatial fast | Instant |
| Offline banner | Network change | Slides down and pushes the list | Spatial default | Instant |

### Animation level setting

S53 offers **Follow system** (default), **Full**, **Reduced** and **Off**. Follow system reads the OS setting (Android animator duration scale, web `prefers-reduced-motion`, iOS Reduce Motion). Reduced swaps the expressive springs for the standard set, removes overshoot and replaces all travel with short fades. Off makes every change instant.

### Performance budget

Animations hold 60 fps on a mid-range phone and use the display's full rate where available. In the list, the 95th-percentile frame time under a swipe is a release check (see the build order). On the web, only compositor properties animate and long lists are virtualised.

### Added in T12
| Animation | Trigger | What moves | Token | Reduced-motion substitute |
|---|---|---|---|---|
| Try it stage (S53) | Press a Try it button | Square pops from 70 percent to 100 percent | Spatial default | Short fade (Effects default); Off: nothing |
| Appearance drill-down (S50 to S51, S52, S53) | Open or close | Shared axis X | Spatial default (move), Effects default (fade) | Cross-fade; Off: instant |

### Added in T13
| Animation | Trigger | What moves | Token | Reduced-motion substitute |
|---|---|---|---|---|
| New row tint | A message arrives while visible | Primary tint fades over 1.2 s (linear) | `CroftTimings.NEW_ROW_TINT_MS` | Same at Reduced; not shown at Off |
| Failed edge | A change the server refused | Red edge on the row, then fades after 2 s | Effects default | Edge removed at 2 s without a fade |
| Dialog open (as built) | Open | Scale from 90 percent and fade | Effects default | Fade only; Off at once |
| Button check | Loading ends | Spinner, then a check for 600 ms | none | Same |
