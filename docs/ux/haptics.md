## Haptics

Haptics confirm a moment in a gesture; they never carry information alone and are never used for passive events (mail arriving while the list is open makes no vibration). The rule is one haptic per gesture moment, and the app asks the system's own haptic API, so the person's system touch feedback setting is respected.

### Levels

S53 sets the level. **Off** sends none. **Subtle** keeps only the moments that carry meaning: swipe threshold, long-press, errors and send. **Full** adds ticks, toggles and selection. The default is Subtle on every platform; Full is always the person's choice.

### Event map

| Event | Level | Android | iOS | Web |
| --- | --- | --- | --- | --- |
| Swipe reaches threshold (arm) | Subtle | `GESTURE_THRESHOLD_ACTIVATE` (API 34); `CLOCK_TICK` below | Impact, medium | 10 ms pulse on supporting Android browsers |
| Swipe leaves threshold (disarm) | Full | `GESTURE_THRESHOLD_DEACTIVATE` (API 34); none below | Impact, light | none |
| Swipe commits | Full | `CONFIRM` (API 30); `KEYBOARD_TAP` below | none (arm already fired) | none |
| Action not available | Subtle | `REJECT` (API 30); `LONG_PRESS` below | Notification, error | none |
| Send accepted | Subtle | `CONFIRM` | Notification, success | none |
| Send failed, or sync error | Subtle | `REJECT` | Notification, error | none |
| A change the server refused (row returns) | Subtle | `REJECT` | Notification, error | none |
| Enter selection mode (long-press) | Subtle | `LONG_PRESS` | Impact, medium | none |
| Context menu opens (long-press, right-click) | Subtle | `LONG_PRESS` | Impact, medium | none |
| Add or remove a row in selection | Full | `SEGMENT_TICK` (API 34); `CLOCK_TICK` below | Selection changed | none |
| Star on or off, switch on or off | Full | `TOGGLE_ON` or `TOGGLE_OFF` (API 34); `CONTEXT_CLICK` below | Selection changed | none |
| Pull to refresh reaches threshold | Full | `GESTURE_THRESHOLD_ACTIVATE` | Impact, light | none |
| Fast scroller moves to next date or letter | Full | `SEGMENT_FREQUENT_TICK` (API 34); `CLOCK_TICK` below | Selection changed | none |
| Start dragging an item to reorder | Full | `DRAG_START` (API 34); `LONG_PRESS` below | Impact, medium | none |
| Item dropped after a drag | Full | `GESTURE_END` (API 30) | Impact, light | none |
| Each slot passed while reordering, or each 15 minutes while dragging an event | Full | `SEGMENT_TICK` | Selection changed | none |
| Slider step (text size, density) | Full | `SEGMENT_TICK` | Selection changed | none |
| Undo tapped | Full | `CONTEXT_CLICK` | Impact, light | none |
| Long-press on Send (schedule) | Subtle | `LONG_PRESS` | Impact, medium | none |
| Destructive dialog confirmed | Subtle | `CONFIRM` | Notification, warning | none |

The API 34 constants and their fallbacks come through `HapticFeedbackConstantsCompat` in AndroidX, so the app writes one call and older phones degrade gracefully. On the web the Vibration API exists only in some Android browsers and iOS Safari has no equivalent, so the web setting is off by default and the web app never depends on it.

### Implementation

- One `Haptics` interface per UI layer with semantic events (`ThresholdArmed`, `ThresholdDisarmed`, `Committed`, `Rejected`, `ToggleOn`, `ToggleOff`, `SelectionStart`, `SelectionTick`, `Tick`, `DragStart`, `Success`, `Failure`). Screens call the events; only the platform file knows the constants.
- A central level filter drops events the current level does not include, and a guard drops the same event if it fires again within 40 ms.
- Tests use a recording implementation: a swipe that crosses the threshold, comes back and crosses again must record Armed, Disarmed, Armed, then Committed, in that order.
- New-mail notification vibration belongs to the Android notification channel, which the person controls in system settings; the app does not override it.
- S53 has Try it buttons that fire each event so the person can feel the difference between Subtle and Full.
