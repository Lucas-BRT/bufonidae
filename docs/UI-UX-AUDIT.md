# Desktop UI/UX audit and implementation

Recorded: 2026-11-17. Branch: `fix/desktop-visual-consistency`.
Source baseline: commit `6162fe8` plus the existing, uncommitted desktop work.
The baseline was not visually captured; no before/after screenshot is claimed.

## Scope and evidence boundary

This audit follows Ferris Atelier's plan, review and improve workflows, with
motion, feedback, perception, layout, navigation, data, content, input and
accessibility guidance. The application remains native Rust/GPUI `=0.2.2`,
gpui-component `=0.5.1`, Wayland and X11 enabled, with its existing warm neutral
palette, red accent and system font. Rust: 1.99.1; host kernel:
Linux 7.2.15-76070215-generic.

Source inspection, compilation and automated behavior tests have been performed.
Interactive findings are not claimed: this session has neither `DISPLAY` nor
`WAYLAND_DISPLAY`, and no Xvfb, screenshot or pointer-automation utility was found.
Orca exists, but there is no graphical application session in which to exercise
it. Native app control is not available through the connected tools.

## The nine phases

| Phase | Delivered | Remaining acceptance evidence |
|---|---|---|
| 1. Baseline | Framework/platform inventory and source baseline recorded | Screenshots/recordings of both themes at minimum, normal and maximized sizes |
| 2. Action/state map | Matrix below; transfer stage messages and cancellation state implemented | Exercise every state in the running product |
| 3. Motion | Shared entrance/exit/progress tokens, activity pulse, result and saved-state fades, persisted reduced-motion control | Observe interruption, frame pacing and preference changes |
| 4. Long-running feedback | Waiting/connecting/planning/transferring distinguished; completed-byte progress labeled; queued/connecting cancellation supported | Real slow/offline/quota scenarios and network-byte progress |
| 5. Surface review | Files/browser keyboard controls, Settings mutation guard, history refresh/details, verification cancellation, library-load retry | Full visual walkthrough of all screens/dialogs |
| 6. Layout/accessibility | Viewport-bounded dialogs, modal focus capture/return and Tab containment; named file controls; theme contrast sample | Resizing, enlarged text, actual focus behavior and Orca |
| 7. Failure/rapid interaction | Tests for queued cancellation, stale config writes, progress interruption and existing failure suite | Interactive matrix below, rapid-click and live network cases |
| 8. Findings | Prioritized register and concrete acceptance conditions below | Close runtime validation items with evidence |
| 9. Implementation/verification | Changes implemented; formatting, Clippy and workspace tests pass | Graphical comparison, accessibility and performance checks |

The graphical acceptance portions of these phases remain open, rather than
being counted as passed because a package compiled.

## Action/state contract

| State | Required representation | Current implementation |
|---|---|---|
| Available | Named action, keyboard activation, visible focus | Toolkit buttons and labeled selection controls |
| Waiting | Waiting status, no invented percent, cancel without waiting for the active operation | Transfer stage event; cancellable daemon lease acquisition |
| Connecting | Account connection status and cancellation | Transfer/verification stage event; cancellation races session checkout |
| Planning | Explain that files/destinations are being checked | Transfer stage before planning; no bar until totals exist |
| Running | Activity, measured completed bytes, file/copy counts | Pulsing activity or static equivalent; live rclone byte/speed counters and completed-file verification counts |
| Cancelling | Acknowledge once; keep a visible disabled cancel control until terminal result | Upload/download and deep-verification stopping states |
| Complete | Persistent result and distinguishable confirmation | Result fade, icon/text and persisted transfer history |
| Partial | Keep completed work; retain actionable per-file errors | Error text persisted in jobs and available through Details |
| Failed | Preserve inputs and expose recovery | Form errors, history actions, library-load retry |
| Interrupted | Review before retry; do not pretend to resume bytes | Persisted history and explicit reviewed upload/download retry |

## Prioritized findings and corrections

“Implemented” below means code and build evidence; visual acceptance is still
pending unless a specific automated test is named.

| ID | Priority | Reproduction / defect | Correction and acceptance |
|---|---|---|---|
| UX-01 | P1 | Start a transfer behind another operation: dialog originally looked active without identifying the queue | Explicit waiting stage; queued cancellation returns without accessing accounts. Automated cancellation test passes |
| UX-02 | P1 | Start a large file: zero percent could look stalled and suggest live-byte measurement | Activity distinguishes connection/planning/work; bar hidden before totals; live counters sampled every 250 ms with explicit completed-file fallback |
| UX-03 | P1 | Cancel deep verification: its cancel action disappeared immediately, before stopping | Visible disabled Cancelling state persists; queue and connection stages also cancel; queued test passes |
| UX-04 | P1 | Enter followed by blur or rapid settings changes could overlap saves or report stale success | One Settings mutation at a time, controls disabled while saving, unchanged limit not rewritten, edits clear prior saved feedback; use returned persisted config |
| UX-05 | P1 | Open Transfers and wait: state required manual refresh | Lightweight job-only refresh every two seconds while open, one request at a time; refresh failure preserves previous results |
| UX-06 | P1 | A partial transfer's modal closes: generic persisted failure loses the actionable reason | Persist per-file failure/conflict/no-room text; Details exposes untruncated error and source/destination information |
| UX-07 | P1 | Keyboard users encounter folder names implemented only as clickable divs | Files uses folder buttons; browser uses labeled checkboxes and a separate Open action |
| UX-08 | P1 | Modal keyboard navigation could reach obscured controls; closing did not explicitly return focus | Capture original focus before opening; contain Tab traversal; restore after exit. Input shield and key capture prevent actions on a closing modal |
| UX-09 | P2 | Abrupt dialog/state changes lack continuity | 160 ms entrance, 120 ms exit, 180 ms measured-progress tween; terminal results and saved confirmation fade once; stable animation identities |
| UX-10 | P2 | Animation has no user-controlled alternative | Persist Reduce motion; disable application fades, tweening, activity pulses and application-owned loading-button spinners; textual status remains |
| UX-11 | P2 | Fixed dialog widths can exceed a reduced viewport | Bound every dialog to viewport width minus 32 px; retain the common vertically scrolling modal shell |
| UX-12 | P1 | Initial library load failure had no direct retry | Retry loading library action added |
| UX-13 | P1 | User reported file-picker rows not filling the available width; Checkbox applies style inside its own outer wrapper | Apply flex growth to a direct row container; set explicit full width on rows, list, body and picker sections; keep folder actions/icons from shrinking. Desktop compilation passes; visual acceptance remains pending |

### Follow-up: animation visibility

The user confirmed launching through `cargo run`, so the older installed system
binary does not explain the report. Navigation had no transition: existing
animations only covered dialogs, terminal results and activity/progress. Added a
200 ms page entrance (fade plus 8 px offset), keyed by navigation generation so
history refresh and background state updates do not replay it. Motion preferences
are now initialized before the first window, rather than relying on the async
Settings load to enable animations. The saved reduced-motion flag is respected.
The actual desktop binary was built successfully; visual acceptance is pending.

### Follow-up: live transfer progress

Uploads/downloads now reset the exclusively leased daemon's counters before
starting, then poll `core/stats` every 250 ms concurrently with the transfer.
The UI displays transferred bytes and measured speed, and smooths the bar between
samples. Retried bytes may exceed the original plan; the running bar is capped
at 99% until the terminal result and never substitutes for integrity verification.
If telemetry is unavailable, transfer/cancellation continue and the UI falls back
to completed-file progress. A throttled 2 MiB local-backend integration test
observed nonzero intermediate bytes before completion and verified counter reset
between operations. The test passed with the real pinned rclone. Graphical/live
Mega verification remains pending. API: https://rclone.org/rc/#core-stats

## Motion rules and limitations

- Movement acknowledges state or preserves continuity; no navigation delay.
- Entrance/exit keys include modal generation, so progress rerenders do not
  replay entrance. Old exit timers cannot remove a newly opened dialog.
- Progress samples its visible intermediate value before a new completion
  target; it does not restart from the previous target or manufacture bytes.
- Unknown progress remains activity plus text. Speed is reported by rclone; ETA is not fabricated.
- Reduced motion is explicitly configurable in Settings. OS reduced-motion
  integration has not been established for the pinned Linux/GPUI stack.
- gpui-component has internal effects, including a checkbox tick fade; the
  application preference is not claimed to disable every toolkit-owned effect.
- Pointer and keyboard input are shielded during the short exit retention.
- Frame rate, high-DPI behavior and screen-reader semantics remain unmeasured.

## Contrast sample

Ratios below were calculated from the exact opaque sRGB theme tokens using
relative luminance. These six text/background combinations exceed 4.5:1 in
both themes. This is a sampled color check, not an accessibility certification;
focus rings, disabled controls, overlays and actual rendering still need review.

| Foreground / background | Light | Dark |
|---|---:|---:|
| `foreground` / `background` | 15.42:1 | 15.21:1 |
| `muted.foreground` / `background` | 5.54:1 | 7.05:1 |
| `muted.foreground` / `popover.background` | 5.87:1 | 6.36:1 |
| `primary.foreground` / `primary.background` | 4.89:1 | 5.80:1 |
| `sidebar.foreground` / `sidebar.background` | 14.19:1 | 14.41:1 |
| `accent.foreground` / `accent.background` | 8.37:1 | 11.58:1 |

## Verification performed

- `cargo check --locked -p bufonidae-desktop`: passed on the Linux host.
- `cargo fmt --check`: passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo test --locked --workspace`: 145 passed, zero failed; nine ignored
  (one desktop Secret Service test and eight generated documentation examples).
- `git diff --check`: passed.
- Tests include old-config compatibility and persisted reduced motion,
  rollback on external config edits, preference restore, zero/unknown progress,
  interpolation interrupted by a new target, queued verification cancellation,
  persisted transfer recovery, and the existing local-rclone integration suite.
- Dependency tooling reports an existing future-compatibility warning for
  `proc-macro-error2`; no dependency version was changed for this work.

## Interactive regression matrix — not run

Run each applicable scenario in both themes. Record environment, build,
viewport, scale, steps, observed outcome and evidence filename. Never use real
user data for destructive/failure experiments.

| Surface | Scenarios | Acceptance |
|---|---|---|
| Files | Empty library, no accounts, nested folders, long/unicode names, 10k entries | Next action discoverable; labels readable; folder and download actions work by keyboard |
| File browser | Select files/folders, open/back, symlinks, permission denial, rapid selection | Selection preserved; Open differs from Select; invalid items cannot be stored |
| Accounts | Loading, unknown space, zero free bytes, offline account, expired login, remove | Unknown is not zero; account failures isolated; removal consequences truthful |
| Settings | Enter+blur, invalid GB, rapid rule changes, write failure, reopen | No overlapping save; failed edit preserved; saved feedback matches disk |
| Motion preference | Enable, restart, toggle off, operate with animations reduced | Preference persists; no app-owned pulses/fades; all statuses still legible |
| Dialogs | Escape, Tab/Shift+Tab, fast open/close, reopen during exit, Enter | Focus stays inside; input cannot reactivate closing form; focus returns to origin |
| Upload/download | Waiting, connecting, large file, many tiny/zero-byte files, partial errors | Stage matches operation; progress never invents network bytes or ETA |
| Cancellation | During queue, connection and transfer; repeated cancellation | Acknowledged once, eventually terminal; completed files preserved |
| Verification | Cost disclosure, no hash, corrupt replica, missing account, cancel | Distinct verified/unverified/error results; no false integrity success |
| Transfers | Job changes, refresh failure, invalid saved request, full details, retry | History refreshes without losing scroll; full errors reachable; no false byte resume |
| Startup | Index locked, corrupt settings/index, missing rclone/keyring | Actionable error, retry where meaningful, no silent replacement |
| Desktop environment | 720x480, 1040x680, maximized, 125/150/200% scale, enlarged text | No inaccessible controls or clipping; dialogs scroll |
| Assistive technology | Orca, keyboard only, contrast/focus in both themes | Names, roles, state and focus are actually exposed; report unsupported semantics |
| Performance | Repeated runs with representative lists/transfers, rapid animation interruption | Measure input/frame timing with hardware/build recorded; no unsupported performance claim |

## Complete Ferris Atelier UX catalog review

All 13 UX modules and the UX routing index were read. Their recipes are
contextual guidance, not a requirement to add every possible feature. Existing
healthy architecture, toolkit focus/hover/pressed rendering, the brand palette
and English source strings are retained. The following table distinguishes
necessary corrections from optional product choices.

| Module | Necessary application / current evidence | Optional choice requiring user selection |
|---|---|---|
| Perception | Related controls stay grouped; actions retain familiar names; selection now has a check icon/current-state text as well as color | Denser or more spacious grouping |
| Layout | Existing viewport bounds/full-width file rows retained; retry actions grouped below errors; sidebar layout retained around native keyboard buttons | Compact/comfortable density setting, resizable sidebar |
| Typography | System font and existing hierarchy retained; explicit units/plurals remain; long text in transfer Details is available | User text-size control and tabular number styling |
| Color | Semantic danger/success/status colors plus text/icon redundancy; existing six-pair contrast sample retained | Manual light/dark override or alternate accent |
| Iconography | Coherent toolkit icons and labeled actions retained; current copy choice gains a check icon | Additional icon/text customization |
| Navigation | Sidebar destinations now use keyboard-operable native buttons with focus and native selected styling; dialog focus rules retained | Back/forward history, extra shortcuts, command palette |
| Input | Shared clickable-control initialization sets pointer cursor; disabled feedback restores arrow; inputs retain native text cursor; busy actions actually disabled; invalid sign-in fields receive focus | Search/filter/multi-selection and context menus |
| Feedback | Existing live bytes/stages/results retained; Settings and folder errors now retry in place; stale folder results cannot overwrite a newer request | OS notifications, tray badge, optional sounds |
| Motion | Existing purposeful transitions, interruption handling and saved reduced-motion setting retained | Motion-intensity setting; automatic OS preference integration needs toolkit/platform investigation |
| Data | Existing stable job ordering and unknown-vs-zero distinctions retained; file-picker control IDs now follow source paths instead of row indices | User sorting/filtering and advanced quota dashboards |
| Content | Explicit Retry loading settings/Retry reading folder actions; current copy count described in plain text; no invented product claims | Guided first-use walkthrough or contextual help panels |
| Accessibility | Named native buttons/checkboxes, focus rings and keyboard activation reused; navigation no longer depends on mouse-only menu controls | Text-size preference; actual Orca verification remains required work, not an optional accessibility exemption |
| Localization | Unicode paths and comma/dot GB input behavior retained; source strings remain English as required by the project | Full translations, locale-specific number/date display and RTL layout |

### Implemented in this follow-up

- `interaction.rs` centralizes native button/checkbox initialization and disabled
  pointer feedback. It does not replace toolkit hover, pressed, focus, click
  suppression or input editing behavior.
- Native sidebar buttons replace only the mouse-oriented menu body; the
  existing sidebar/header/groups/footer remain. The active destination uses native selected styling without a textual suffix, per user preference;
  each destination remains keyboard-operable and has its normal identifying icon.
- The file picker uses path-derived control IDs, shows a Reading folder state,
  preserves selection, rejects stale async results, and retries the exact failed
  path. Navigation to the parent is disabled while loading.
- Settings can retry failed configuration reads. Copy choice uses native
  selected state and a check icon, with a plain-text description of the current
  default. Existing values are not overwritten after a failed read.
- Busy sign-in, removal and account-check controls are disabled rather than
  merely displaying a loading icon; local form validation focuses the field
  needing correction without changing its value.

### Optional backlog — intentionally not enabled

1. Compact/comfortable density and adjustable text size.
2. User-controlled light/dark mode instead of only following the system.
3. Search, sorting, filters and multiple selection.
4. OS completion/error notifications, tray badge and optional sounds.
5. Additional shortcuts, context menus and a command palette.
6. Translations, locale formatting and RTL.
7. Onboarding tour and contextual help.

These are product choices beyond the basic interaction corrections. The user
can select any subset; they are not mutually exclusive. Actual native visual,
keyboard, IME, screen-reader, scaling and performance acceptance still require
an interactive graphical session. No new visual acceptance is claimed here.

## Remaining product work

This audit does not complete the wider storage roadmap: durable per-file resume,
automatic recovery scheduling, rename/move/delete,
backup/import UI, packaging qualification and live-Mega validation remain tracked
in [PLAN.md](PLAN.md). Responsive browser review of the earlier website command
reference change also remains pending.

## Selected desktop preferences: 1, 3, 4, 5 and 7

The user selected density/text, file browsing/selection, notifications,
productivity controls and help. The active sidebar label no longer includes
`(current)`; selected styling alone marks the active destination. Manual theme
selection and full translations were not selected.

### Implemented behavior

- Settings has separate Density and text, Search/sorting/selection,
  Notifications and sounds, Keyboard/context menus, and Guidance/help sections.
  These are editable drafts with Save desktop preferences and Discard unsaved
  changes. They are distinct from the existing immediately saved redundancy
  and upload-limit settings. Failed persistence preserves the draft and does
  not change live preferences. Legacy config files receive defaults.
- Files has current-folder substring search, case policy, name/size ordering
  in both directions, optional folders-first grouping and All/Files/Folders
  filters. Sorting has deterministic path tie-breaks. Checkboxes select paths;
  Select/Deselect visible only affects current results, Clear removes all
  selections, and filtering does not silently discard hidden selections.
- Multiple downloads preserve virtual paths, subsume descendants of selected
  folders and retain existing files. Single-path downloads keep their prior
  output naming. Saved new download requests store a path vector; history
  decoding remains compatible with older single-path requests.
- Density changes Files rows (52/40 px); base text size is independently chosen
  from 12/14/16/18/20 px and follows system appearance changes without resetting.
- Notification settings independently select success, partial/failure and
  cancellation events, 3/5/10-second requested duration, whether paths may appear,
  and Silent/Errors only/All enabled event sounds. Separate test buttons preview
  completion and failure using unsaved options. Repeated test clicks are blocked
  until the request returns. Paths are omitted by default and markup is escaped.
- Notifications use the standard Linux desktop service through `gdbus` without
  a shell; sound uses `paplay` and the installed freedesktop sound files. Delivery
  has timeouts and failure details. Transfer completion is not delayed by native
  notification delivery, and notification failures do not turn a successful
  transfer into a failed transfer. OS do-not-disturb/final timeout policy remains
  under desktop control. The app must remain running.
- Full/Navigation only/Off shortcut presets are documented in Settings. Full
  includes Ctrl+1…5, Ctrl+F, Ctrl+O, Ctrl+K, F1, F5 and Alt+Up. Navigation only
  keeps screen navigation, refresh and parent-folder navigation. Custom commands
  do not override input in open dialogs. Standard Tab/Enter/Escape remain.
- Commands is available as a sidebar button as well as Ctrl+K. Its search,
  Up/Down and Enter activation lead to screens and available product actions.
  Files context menus expose Download and Copy virtual path and can be disabled.
  Shortcut hints are independently configurable.
- Help remains available as a sidebar button; full help covers accounts,
  storing, selection/download, recovery, notifications/privacy and preferences.
  Its optional five-step tour offers Previous/Next and explicit links to the
  relevant screen/action. First-account and Files explanations are configurable;
  hiding them never hides errors, required confirmations or verification costs.

### Native tray follow-up

The previous tray limitation is resolved in code using exactly pinned `ksni
0.3.7`, a pure Rust StatusNotifierItem implementation on the existing desktop
Tokio runtime. This avoids a GPUI fork and keeps platform integration outside
bufonidae-core. Settings offers four independent saved controls: enable the
icon, retain it while idle, show a count overlay, and open Transfers on primary
click rather than the current screen. Its native menu opens the app, Transfers
or Settings. Navigation preserves active dialogs. Counts include queued/running
jobs, refresh every two seconds, cap at 99 in the overlay and remain exact in
the tooltip. Disabled tray does not connect to the session bus.

Startup failures, missing watcher support and disconnected handles are reported
in Settings with explicit Retry; enabling the icon never claims it was visibly
rendered. GNOME may require an AppIndicator extension. The shell controls
whether overlay pixmaps are shown. Closing the window still exits the app.
Visual acceptance on a real desktop remains pending.

### Evidence

`cargo test --locked --workspace`: 146 passed, none failed, ten ignored.
The extra ignored test requires an isolated session bus; run it with
`dbus-run-session -- cargo test --locked -p bufonidae-desktop missing_watcher_is_reported_and_disabling_recovers -- --ignored`.
It exercises missing watcher feedback and disabling recovery without touching
the user's session bus. The isolated D-Bus test passed (one test).
Tray unit coverage also checks overlay bounds/capping, hidden badges at zero,
exact tooltip counts and primary-click destinations. Desktop build, formatting
and workspace Clippy with warnings denied passed.
New regression tests cover preferences persistence/validation/rollback/restore,
multiple-path download de-duplication and path preservation, search/filter/sort
choices and Unicode names, and safe notification argument encoding. Clippy with
warnings denied and formatting checks passed. Native desktop notification/audio
delivery, tray support, actual visual resizing and Orca remain unverified; the
notification test buttons are provided for execution in the user's session.

### Transfers and sidebar layout correction

The transfer virtual list and its rendered rows lacked explicit full-width
constraints; both, the header and the details panel now use the available width.
The old SidebarFooter lays children horizontally: two command buttons plus the
library summary exceeded the sidebar width. Its single child is now a vertical,
full-width storage panel. Commands/Help live in a persistent utility bar above
the selected screen, preserving discoverability and keyboard shortcuts.

Ferris Atelier layout guidance applies to width allocation and task priority;
navigation guidance applies to stable utility action placement; data guidance
applies to labeled byte units, truthful percentages and explicit missing data.
The quota panel includes actual remotely reported used/total/available bytes,
account coverage, a progress bar and explicit Refresh. A pulse marks checking
when motion is enabled. Aggregate totals exclude incomplete/unreachable reports
and accounts no longer in the current library. Over-quota use is preserved in
the numbers while the bar is bounded at 100%; available bytes never go negative.
Per-account errors remain available in Accounts. The panel labels a completed
query as the last check rather than implying continuously live quota data.

Regression tests exercise partial coverage, deriving missing used/total values,
over-quota storage, empty results and arithmetic overflow. Manual acceptance
remains pending: narrow/large windows, 12–20 px text, long transfer names/errors,
full-width row separators and right-aligned actions, utility buttons, sidebar
footer, loading/partial/error states, keyboard operation and reduced motion.

Verification for this follow-up: 148 workspace tests passed, zero failed,
ten ignored (including the isolated-bus tray case). Desktop build, workspace
Clippy with warnings denied, formatting and diff whitespace checks passed.
No graphical acceptance or live account quota query was performed here.

### Dedicated storage distribution and Sync

Ferris Atelier data principles: current/projected quota bars use the same 0–100%
scale per account, with explicit byte figures and indexed-versus-remote usage.
Partial quota/account failures stay visible and block destructive planning.
Traffic includes PC staging and readback/recheck costs rather than assuming a
remote-to-remote transfer. Configured caps are explicit assumptions; the hours
formula and a separate 50%-speed scenario make uncertainty visible. Provider
quota/reset remains unknown; entered reset delay is honored before transfers.

Navigation/input: the entire meter uses a native button with hover, focus,
keyboard activation, tooltip and selected state. Refresh is a separate button,
never nested inside the meter. Storage is also searchable in Commands and F5
refreshes it. Strategies and numeric controls invalidate the previous preview
and approval; controls are locked while loading/planning/running. A virtualized
file-change list exposes additions/removals, with Details for full paths.
Approval is separate from generating a read-only preview. Future upload policy
stays in Settings; operation controls apply to this page/run.

Feedback/recovery: checking/planning use the existing reduced-motion-aware
activity indicator. Execution reports real byte totals/speed and file/stage
labels, with the shared 180 ms progress interpolation capped below completion
until the operation finishes. Unavailable byte telemetry falls back to completed
files. Stop explicitly says it waits for the current file; successful earlier
files remain committed. Completion refreshes distribution and sidebar quota;
root job polling includes this screen. Errors retain settings and direct users
to Check and a fresh preview. Sync history does not offer an unsafe blind retry.

Implementation verification covers plan traffic/duration and staging counts,
policy floors, preview budgets, unsafe/stale paths, missing workspace, unchanged
balanced layouts, corrupt uploads preserving the original, original content
changes before removal, cancellation, existing-target refusal, commit/removal
ordering and cleanup. A real rclone alias/local test executes the full staged
round trip and verifies the resulting index/source/destination. Additional real
rclone tests validate IgnoreExisting, separate byte-rate caps, staging disk
availability and deletefile. These fixtures use temporary local directories,
not live Mega accounts. GUI, keyboard/screen-reader acceptance and live-Mega
qualification remain pending in a graphical session.

Final verification for distribution/Sync: 158 workspace tests passed, zero
failed, ten ignored. The real rclone round-trip/commit/removal test and the RC
endpoint contract test both ran and passed. Desktop build, workspace Clippy
with warnings denied, formatting and diff whitespace checks passed. No live
Mega mutation was performed; graphical acceptance remains pending because this
session has no DISPLAY/WAYLAND_DISPLAY.
