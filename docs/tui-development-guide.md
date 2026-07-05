# A Deep Dive into TUI Development: From zellij-workbench to lazygit

*[中文版](tui-development-guide.zh-CN.md)*

This guide uses two real codebases — this repo (`zellij-workbench`, Rust +
[ratatui](https://ratatui.rs)) and [lazygit](https://github.com/jesseduffield/lazygit)
(Go, which vendors its own fork of
[gocui](https://github.com/jesseduffield/lazygit/tree/master/pkg/gocui) built on
[tcell](https://github.com/gdamore/tcell)) — to break down the common patterns
in terminal UI (TUI) development: how to design the event loop, how to get
rendering fast, how to organize layout, and what a "good" TUI actually needs
to have.

Every code reference in this guide has a real source: `zellij-workbench`
references point straight at `src/tui.rs`; lazygit references cite specific
file paths and approximate line numbers (from a clone of
[jesseduffield/lazygit](https://github.com/jesseduffield/lazygit) at a
particular point in time — line numbers may drift slightly as upstream
changes, but the structural conclusions are stable).

## Table of Contents

1. [Two Architectural Paradigms: Immediate Mode vs. the Elm Architecture](#1-two-architectural-paradigms-immediate-mode-vs-the-elm-architecture)
2. [The Event Loop: A TUI's Heartbeat](#2-the-event-loop-a-tuis-heartbeat)
3. [Rendering Performance: Immediate Mode Doesn't Mean Brute-Force Redraws](#3-rendering-performance-immediate-mode-doesnt-mean-brute-force-redraws)
4. [Layout: Constraint-Driven, Not Pixel Math](#4-layout-constraint-driven-not-pixel-math)
5. [State and Navigation: From an Enum to a Context Stack](#5-state-and-navigation-from-an-enum-to-a-context-stack)
6. [Async and Responsiveness: Never Block the Event Loop](#6-async-and-responsiveness-never-block-the-event-loop)
7. [The Essentials Checklist: What a "Good" TUI Needs](#7-the-essentials-checklist-what-a-good-tui-needs)
8. [Side-by-Side Comparison](#8-side-by-side-comparison)
9. [Further Reading](#9-further-reading)

---

## 1. Two Architectural Paradigms: Immediate Mode vs. the Elm Architecture

Before writing a TUI, settle one question: what's the relationship between
your UI state and your rendering logic? The industry has largely converged
on two answers.

**Immediate Mode** — ratatui's own docs put it plainly: "the UI is
recreated every frame; you draw it from scratch based on current state,
with no persistent widget objects." The typical shape:

```rust
loop {
    terminal.draw(|f| {
        if state.condition {
            f.render_widget(SomeWidget::new(), layout);
        } else {
            f.render_widget(AnotherWidget::new(), layout);
        }
    })?;
}
```

`zellij-workbench`'s `draw_tui` (`src/tui.rs`) is exactly this: every call
to `terminal.draw(|frame| {...})` rebuilds `List`, `Paragraph`, `Line`, and
other widgets from scratch, holding no reference to last frame's widgets.
The upside is directness — the UI logic is a straight projection of state,
so you never hit "widget-internal state drifted out of sync with app
state" bugs. The cost is that you own the render loop and the event loop
yourself; the library won't decide "when to redraw" for you.

**The Elm Architecture (Model-Update-View, MVU)** — Go's
[bubbletea](https://github.com/charmbracelet/bubbletea) (43k+ stars) is the
flagship of this school. Three functions form the core:

- `Init() Cmd` — returns the initial command to run at startup
- `Update(Msg) (Model, Cmd)` — takes a message, returns new state plus an
  optional side-effect command
- `View() string` — a pure function that reads the Model and renders a
  string

Data flows in one direction: `event → Msg → Update → new Model → View →
terminal`. Side effects (HTTP, timers, subprocesses) are isolated inside
`Cmd`, and once they complete they turn back into a `Msg` fed to `Update`.
This whole design is essentially "how do you safely fold an async result
back into state," codified as a framework contract via the type system.

**gocui/lazygit is a third, more pragmatic shape**: a `View`
(`pkg/gocui/view.go`) is a **persistent object** holding its own line
buffer, cursor, and scroll position — much closer to the traditional
retained-mode GUI notion of a "control." But rather than waiting for
events to update properties incrementally, it's driven by a `tainted`
dirty flag plus explicit `SetContent`/`Write` calls — effectively
"stateful immediate mode": the view object persists, but its content is
still replaced wholesale each time rather than patched incrementally.

**Recommendations**:

- If you're writing Rust, ratatui's immediate mode is close to the default
  choice — embrace it directly. Don't try to hand-roll another "widget
  tree + diff" layer on top; the library's own `Buffer` diffing (see
  section 3) already solves the performance problem.
- If your app has a clearly message-driven shape (network requests,
  subscriptions, multiple independent async sources), the Elm Architecture
  saves you from a lot of "where did this state get mutated" bugs. Even
  without bubbletea, you can hand-roll a simplified version inside ratatui
  (`zellij-workbench`'s `mpsc` background-refresh channel is essentially a
  simplified `Cmd → Msg` channel — see section 6).
- If your app has "panels + popups + multi-level navigation" complexity
  (like lazygit), you'll eventually need a persistent view object plus
  explicit dirty flags — purely functional "rebuild everything every
  frame" gets unwieldy once the state volume grows large enough.

---

## 2. The Event Loop: A TUI's Heartbeat

Whichever paradigm you pick, the event loop skeleton looks the same:

```text
loop {
    read/wait for an event (keyboard, mouse, resize, timer, background-task completion)
    update state based on the event
    re-render as needed
}
```

**`zellij-workbench`'s implementation** (`src/tui.rs::draw_tui`):

```rust
loop {
    apply_completed_refresh(&refresh_rx, ..., &search, view, server_filter.as_deref());

    if last_auto_refresh.elapsed() >= AUTO_REFRESH_INTERVAL && !auto_refresh_in_flight {
        spawn_auto_refresh(refresh_tx.clone());
        auto_refresh_in_flight = true;
        ...
    }

    terminal.draw(|frame| { /* render the whole UI tree */ })?;

    if event::poll(Duration::from_millis(200))? {
        if let Event::Key(key) = event::read()? {
            match mode { /* handle the key */ }
        }
    }
}
```

Three key design points:

1. **`event::poll(Duration::from_millis(200))` instead of a busy loop**:
   a blocking `event::read()` would leave you unable to check "is it time
   to auto-refresh" while waiting for a keypress; an uncapped busy-poll
   loop (`loop { if let Ok(ev) = try_read() {...} }`) would pin a CPU core
   at 100%. `poll(timeout)` splits the difference: wait at most 200ms —
   either an event arrives early, or the timeout fires and the loop goes
   back to the top to check periodic work like auto-refresh. 200ms is
   imperceptible latency for a human keypress, but infrequent enough to
   not waste CPU.
2. **Render happens before waiting for events**: every iteration calls
   `terminal.draw` first, then waits for an event — this guarantees "any
   state change (even one a background thread just pushed in) gets drawn
   on the very next loop iteration," with no extra "is it dirty" check
   needed (ratatui itself diffs at a lower level — see section 3).
3. **The event loop is single-threaded; side effects go elsewhere**: the
   entire `draw_tui` function runs on one thread. Anything that might
   block (scanning zellij sessions, running a git command) is never
   allowed directly in this loop — a 3-second scan would freeze the whole
   UI, including key handling, for 3 seconds. This sets up section 6.

**lazygit/gocui's implementation** (`pkg/gocui/gui.go`) goes one step
further: "two channels feeding one single-threaded event loop."

- The `pollEvent()` goroutine does exactly one thing: push tcell's
  terminal events into `g.gEvents` (a buffered channel).
- Any background goroutine that wants to update the UI must call
  `Gui.Update(f func(*Gui) error)` or `Gui.UpdateContentOnly(f)`, which is
  really just pushing a closure into the `g.userEvents` channel:

  ```go
  func (g *Gui) Update(f func(*Gui) error) {
      task := g.NewTask()
      select {
      case g.userEvents <- userEvent{f: f, task: task}:
      default:
          panic("gocui: userEvents channel full; refusing to block or reorder")
      }
  }
  ```

  Notice the **deliberately non-blocking `select`/`default`**: if the
  channel is full, it panics outright rather than blocking or silently
  falling back to synchronous execution. The reasoning is practical — a
  blocking send from the UI thread to itself could deadlock, and "just run
  it inline when full" looks safe but actually breaks the ordering
  guarantee callers depend on. Better to panic and surface the bug than to
  silently break ordering.

- The main loop's `processEvent()` does one `select` across both channels,
  then `processRemainingEvents()` non-blockingly drains everything
  currently queued **before deciding whether to render** — this is
  explicit event coalescing: rather than triggering a full render per
  keypress, process a batch of events that arrived close together and
  render once.

**General takeaway**: regardless of language or framework, "single-
threaded event loop + all cross-thread communication goes through a
channel/queue" is close to an iron law for TUIs (and for any stateful
interactive program, really). You can let the type system enforce this
(the Elm Architecture's `Cmd`/`Msg`), or fall back to a runtime panic like
gocui does, but the line "UI state can only be mutated by the thread that
owns it" is not one to cross.

---

## 3. Rendering Performance: Immediate Mode Doesn't Mean Brute-Force Redraws

The most common misunderstanding about immediate mode is "redrawing every
widget every frame = sending every byte to the terminal every frame." In
reality these are two separate things, with a diffing layer in between.

**ratatui's Buffer diffing**: `Terminal::draw()` renders widgets into an
in-memory `Buffer` (a 2D array of cells). At the end of a frame, it
doesn't convert the whole `Buffer` into escape sequences and ship it to
the terminal — instead, `ratatui-core`'s `BufferDiff` iterator only emits
the `(x, y, cell)` triples that **actually changed** this frame, to
whichever backend you're using (crossterm/termion/…):

> `BufferDiff`: a zero-allocation iterator over the differences between
> two frame buffers, yielding only the `(x, y, &Cell)` entries in `next`
> that differ from the corresponding position in `prev`.

**gocui/tcell follows the identical idea, just one layer lower**:
`View.draw()` (`pkg/gocui/view.go`) writes characters into tcell's
`CellBuffer`; `CellBuffer.Dirty(x, y)` compares this frame's and last
frame's `currStr`/`lastStr`, and the `drawCell()` triggered by
`Screen.Show()` checks `if !t.cells.Dirty(x, y) { return }` first —
**cells that haven't changed never make it into the byte stream sent to
the terminal**.

**Takeaway 1: you almost never need to hand-write "dirty rectangle"
logic.** Libraries like ratatui and tcell already diff at the cell level —
that performance is free. What you actually need to worry about is the
following.

**Takeaway 2: the real performance trap is at the "logical rebuild"
layer, not the "physical transmission" layer.** Even though physical
transmission is already filtered by diffing, if your render closure
formats 10,000 strings for a 10,000-row list that isn't even on screen,
that CPU cost has already been paid — diffing only saves the last little
step of "sending it to the terminal." lazygit handles this very
concretely:

- **List virtualization** (`pkg/gui/context/list_context_trait.go::HandleRender`):
  only when `renderOnlyVisibleLines` is true does it use the viewport's
  visible range (`ViewPortYBounds()`) to call
  `renderLines(startIdx, startIdx+length)` — **formatting only the dozen
  or so currently-visible rows**, not the entire commit history or file
  list. The code comment says it plainly: "rendering only the visible
  area can save a lot of memory for lists that can get very long."
- **A "content-only" fast path that skips the whole layout recompute**:
  when a batch of events is entirely tagged `contentOnly`,
  `flushContentOnly()` skips the expensive `Layout()` recompute entirely
  and only redraws views marked `tainted` (plus any view whose area
  overlaps one of those). lazygit uses this for the status-bar spinner
  animation — a spinner ticking every 100ms has no reason to trigger a
  full window-layout repass.
- **Explicit caching of expensive computations**: the commit graph's
  "pipes" (the vertical/diagonal lines connecting commit nodes) aren't
  cheap to compute; `pkg/gui/presentation/commits.go` keys a
  `pipeSetCache` on `(commitHash, commitCount, divergence)` to hold onto
  the result. Likewise, the ANSI-escape-formatted result of a colored
  string is cached in `rgbCache`, and `git config` reads are cached in
  `CachedGitConfig` so a refresh doesn't fork a new `git config` subprocess
  every time.
- **Throttling bursts of requests**: `pkg/tasks/tasks.go` has two small
  but critical constants:

  ```go
  const THROTTLE_TIME = time.Millisecond * 30
  const COMMAND_START_THRESHOLD = time.Millisecond * 10
  ```

  When a user rapidly scrolls through the commit list, every cursor move
  can trigger a `git show`. If the previous command got cancelled before
  it finished (because the user moved again) and the system already looks
  under load (start latency over 10ms but total runtime under 30ms), the
  next command sleeps 30ms before starting — explicitly to avoid "the user
  flicking through pages" turning into "a stampede of forked subprocesses."

**Checklist**:

- Long lists should always be viewport-windowed (format only visible
  rows) rather than relying on the underlying diff to "save you" — diffing
  only saves transmission cost, not the cost of building
  strings/computing display values.
- Anything that's "the same input, recomputed repeatedly in a short
  window" (colorized strings, graph structures, external command results)
  is worth memoizing in a plain `HashMap`.
- High-frequency but visually lightweight updates (progress bars,
  spinners, status text) should have a fast path that doesn't trigger a
  full layout recompute.
- Operations a user can trigger rapidly (paging, search input) that are
  backed by a heavyweight command should "cancel the previous one and
  throttle the next" rather than firing them all off concurrently without
  restraint.

---

## 4. Layout: Constraint-Driven, Not Pixel Math

Good TUI layout systems converge on the same idea: **declarative
constraints/weights, not hand-computed x/y coordinates** — so a terminal
resize only requires re-solving the constraints, with zero changes to
business logic.

**ratatui's `Layout` + `Constraint`**, as actually used in
`zellij-workbench` (`src/tui.rs::draw_tui`):

```rust
let shell = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(3),  // top status bar: fixed 3 rows
        Constraint::Min(8),     // main body: at least 8 rows, gets the rest
        Constraint::Length(1),  // bottom keybinding hint: fixed 1 row
    ])
    .split(frame.area());

let chunks = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
    .split(shell[1]);
```

This shows the typical **nested split**: first slice the whole terminal
vertically into "header/body/footer" (mixing `Length`/`Min`/`Length`),
then slice the middle section horizontally into a 52%/48% two-column split
(list + detail). None of this code needs to change on resize — the next
call to `frame.area()` returns the new size, and `split` re-solves the
constraints against it.

**lazygit's `boxlayout`** (vendored as `lazycore/pkg/boxlayout`) takes the
same road, just structured as a recursively-nestable tree:

```go
type Box struct {
    Direction Direction   // ROW or COLUMN
    Children  []*Box
    Window    string      // leaf node: which window this box represents
    Size      int         // static size (mutually exclusive with Weight)
    Weight    int         // dynamic weight, proportional share of remaining space
}
```

`ArrangeWindows`'s allocation algorithm is essentially flexbox: **satisfy
every child with a fixed `Size` first, then divide the remaining space
among the rest proportionally by `Weight`** — the same model as CSS's
`flex-grow`. `pkg/gui/controllers/helpers/window_arrangement_helper.go::GetWindowDimensions`
rebuilds the entire `Box` tree fresh every frame from the current screen
width/height, focused window, and screen mode (normal/half/full), then
calls `ArrangeWindows` to solve it — **layout is recomputed from scratch
every time, never incrementally patched**, because the tree itself is
small enough that the recompute cost is negligible. This is also how
lazygit gets zero special-case handling for resize: `flush()` detects a
terminal size change and simply reruns `Layout(g)`; the new size is
absorbed naturally by the next layout computation.

**Shared lessons from both**:

1. **The constraint system's complexity should match the layout's
   complexity — you don't need to jump straight to the most sophisticated
   solution.** `zellij-workbench`'s static two-column-plus-header/footer
   layout is well served by `Layout`/`Constraint` alone; lazygit has a pile
   of dynamic rules ("normal/half/full screen," "portrait/landscape
   switching" via `shouldUsePortraitMode`, "accordion mode expanding the
   focused panel") that justify pulling out a dedicated, recursive `Box`
   tree.
2. **Responsiveness isn't "special-casing the resize event" — it's a
   natural side effect of "re-solving layout on every render."** Neither
   codebase has dedicated branch logic for resize, because layout was
   already being computed every frame; size is just one more input
   parameter.
3. **Layout and content stay decoupled**: whether it's the `Rect` array
   returned by `Layout::split` or the `map[string]Dimensions` returned by
   `ArrangeWindows`, both only answer "how big is this region and where is
   it" — never "what gets drawn inside it," which is always decided
   separately during the render pass.

---

## 5. State and Navigation: From an Enum to a Context Stack

`zellij-workbench`'s TUI currently has exactly two "modes":

```rust
enum InputMode {
    Normal,
    Search,
}
```

An enum plus a `match` is entirely sufficient for a flat "browse a list +
type into a search box" interaction — every key-handling branch in the
code is unambiguous.

But once your app needs something like "pressing a key in panel A pops a
menu, the menu pops a confirmation dialog, Esc has to unwind one level at
a time, and unwinding must restore the exact scroll position/selection the
underlying panel had before" — a flat enum rapidly explodes into a giant
`match` over the Cartesian product of every mode.

lazygit chose an **explicit context stack** (`pkg/gui/context.go::ContextMgr`):

```go
type ContextMgr struct {
    ContextStack []types.Context
    sync.RWMutex
    ...
}
```

Every panel/popup implements the `types.Context` interface and declares
its own "kind" (`ContextKind`): `SIDE_CONTEXT` (a side panel, e.g. the
files/branches/commits list), `MAIN_CONTEXT` (the main content area, only
one active at a time), `TEMPORARY_POPUP` (a use-once-and-discard popup,
e.g. a confirmation dialog), `PERSISTENT_POPUP` (a stackable popup, e.g. a
menu), and so on. Push rules branch on kind:

- Pushing a `SIDE_CONTEXT` → **clears the whole stack**, leaving only
  itself (side panels are mutually exclusive, no nesting).
- Pushing a `MAIN_CONTEXT` → removes any other `MAIN_CONTEXT` already on
  the stack, but keeps side panels and other layers underneath (only one
  main view at a time, but it can sit "on top of" a side panel).
- Anything else (menus, confirmations, …) → if the top of the stack is a
  temporary popup, it gets replaced rather than nested (the code comment
  candidly admits this is because temporary popups reuse the same view;
  ideally you'd be able to escape back through previous temporary popups,
  but the current implementation can't — a known, acknowledged trade-off,
  not a pretense of perfection).

This stack is also **the sole source of truth for keyboard event
routing**: activating a context via `ContextMgr.Activate` calls
`gocui.Gui.SetCurrentView(viewName)`, and gocui's `matchView()` (which
decides who handles a given keypress) only ever looks at `g.currentView`.
In other words, the answer to "who does this keypress belong to right
now" is always "look at the top of the context stack" — there's no
separate "currently focused panel" state to keep in sync.

**When to graduate from an enum to a stack**:

- Your "modes" start exhibiting genuine **nesting** rather than simple
  mutually-exclusive switching (a popup that can itself pop another
  popup) — time for a stack.
- The semantics of Esc/back become "return to the previous level" rather
  than "go back to some fixed default state" — a stack expresses this
  naturally; an enum can only simulate it by separately recording "what
  was the previous state."
- Different "modes" need independent keybinding tables, rather than
  enumerating every mode's every key combination inside one giant
  `match` — each context declares its own keybindings, and routing
  naturally follows the stack top at registration time.

**Don't reach for it too early**: if your app genuinely only has
"browse + search" flat interaction (which is exactly where
`zellij-workbench` is today), an enum is far easier to maintain and read
than a full context-stack abstraction. A context stack is for apps that
genuinely need nested navigation — it isn't a TUI starter-kit default.

---

## 6. Async and Responsiveness: Never Block the Event Loop

Section 2 established that the event loop must be single-threaded — so
what do you do about slow operations (network, subprocesses, disk)? Both
codebases give a strikingly consistent answer: **spawn a thread/goroutine
to do the work, send the result back over a channel, and have the event
loop non-blockingly check "is there a new result yet."**

**`zellij-workbench`'s background refresh** (`src/tui.rs`):

```rust
let (refresh_tx, refresh_rx) = mpsc::channel();
...
fn spawn_auto_refresh(refresh_tx: Sender<RefreshResult>) {
    thread::spawn(move || {
        let result = refresh_index_report()
            .and_then(|summary| { /* re-read from SQLite */ })
            .map_err(|err| format!("{err:#}"));
        let _ = refresh_tx.send(result);
    });
}
```

The top of every main-loop iteration calls `apply_completed_refresh`,
which non-blockingly drains `refresh_rx.try_recv()` for any background
task that finished, merges it into UI state, and uses
`selected_workspace_id`/`restore_selection` to preserve the user's
currently selected row by ID rather than resetting the cursor to the top
of the list on every refresh. **Scanning zellij sessions and running git
commands — operations that can take anywhere from a few hundred
milliseconds to a few seconds — run entirely on another thread; the event
loop keeps polling for keypresses on its normal 200ms cadence** and never
stutters just because a scan happens to be in flight. This is, in effect,
a hand-rolled, simplified version of "`Cmd` dispatch + `Msg` collection"
(the Elm Architecture idea from section 1), just without a formal
type-system contract behind it.

**lazygit's two layers of async**:

1. **`Gui.OnWorker(f func(Task) error)`** — spawns a goroutine to run `f`,
   automatically wrapped in panic recovery (so one crashed background task
   doesn't take the whole terminal down with it), and tracked by a
   `TaskManager` that answers "is any background work still in flight
   right now" (used to decide whether to show a "working…" indicator).
2. **`RefreshHelper.Refresh(options)`** — a single "refresh" can touch
   several categories of data at once (file status, branches, commits,
   stash, …); each category runs concurrently via `OnWorker`, and once it
   finishes it **must** route its result back through
   `OnUIThread(func() error {...})` (which is really just `Gui.Update`) —
   a worker goroutine is never allowed to touch a view directly. To
   prevent "the file list hasn't finished refreshing and another refresh
   just started" from stomping on itself, each category of data has its
   own lock (`RefreshingFilesMutex`, `RefreshingBranchesMutex`, …).

**Finer-grained control — `ViewBufferManager` in `pkg/tasks/tasks.go`**:
scrolling through commit history triggers a `git show` for every selected
commit, and the output can be long. Rather than waiting for the whole
command to finish and dumping the output into the view all at once, it's
consumed and displayed incrementally: `NewCmdTask` spins up 4 cooperating
goroutines per command — one scans stdout lines into a channel, one shows
a "loading…" placeholder if nothing has arrived within 200ms, one consumer
writes arriving lines into the view and triggers a partial redraw, and one
watches for a cancellation signal so it can gracefully terminate the
previous, still-running process the moment the user scrolls to the next
commit.

**Checklist**:

- Anything that might block longer than one frame's worth of time
  (roughly 16-100ms, depending on how smooth you want things to feel)
  must never go directly onto the thread the event loop runs on.
- Results computed by a background thread/goroutine must be handed back
  to the event loop's own thread via a channel/queue before mutating
  state — don't reach for a `Mutex<State>` mutated directly across
  threads and hope the render thread "just picks it up." That path can
  technically work, but the mental overhead and potential
  data-race-debugging cost are both far higher than explicit message
  passing.
- If the same kind of refresh can be triggered at high frequency (a user
  mashing a refresh key, rapid paging), either deduplicate ("don't start
  another one if one's already running") or throttle (wait a moment after
  finishing before accepting the next) — otherwise slow tasks pile up
  without bound.
- For commands with long output, consider streaming consumption instead
  of "wait for it to finish, then process it all at once" — it
  meaningfully improves perceived latency on large repos/long histories.

---

## 7. The Essentials Checklist: What a "Good" TUI Needs

Turning the lessons above into a more product-facing checklist — these are
all feature points with concrete evidence in either `zellij-workbench`'s
or lazygit's actual implementation:

- **Keybindings must be discoverable — users shouldn't have to memorize
  documentation.** `zellij-workbench`'s bottom status bar
  (`footer()` / `controls_line()`) permanently shows every key available
  in the current mode; lazygit goes further — `OptionsMapMgr` renders the
  current context's keybindings into a bottom options bar, plus a
  dedicated `?` key that opens a full, searchable keybinding menu
  (`OptionsMenuAction`). **The minimum viable version is just one status
  line listing "keys available right now" — the return on that
  investment is enormous.**
- **Search/filter should support structured queries, not just plain
  substring matching.** `zellij-workbench`'s `SearchQuery::parse`
  (`src/tui.rs`) layers `server:` `status:` `tag:` `git:` prefix syntax on
  top of plain-text keywords, so a single line of input can express a
  compound condition like "workspaces on the prod server, status active,
  git dirty."
- **Async operations need visible status feedback**, even if it's just a
  line of text. `zellij-workbench` has `scan_status` ("Scan: idle" /
  "refreshing..." / "ok, N workspaces" / "failed (...)"); lazygit has a
  spinner. Users need to know whether "it's working" or "it's stuck."
- **Default actions should be conservative; destructive actions should
  require an explicit escalation.** `zellij-workbench` deliberately makes
  `attach` fail with an error suggesting `recreate` when the session is
  missing, rather than silently creating a new empty session on your
  behalf — this avoids quietly "resurrecting" a workspace you thought
  still existed into an empty shell. lazygit pops a confirmation dialog
  before things like force-pushing or discarding changes.
- **Background refresh must not disrupt the user's current state.**
  `restore_selection` (`src/tui.rs`) finds the previously selected row
  back by ID rather than by index after data reloads — with the naive
  "reset selection to zero after every refresh" approach, the user would
  get forcibly bounced back to the top of the list every 30 seconds, which
  is a miserable experience.
- **Resize should require no special handling.** See section 4 — as long
  as layout is "re-solved from the current terminal size every frame," you
  almost never need dedicated resize-handling code.
- **Terminal state must be recoverable on an abnormal exit.** This is a
  real issue discovered in this very repo while writing this guide:
  `run_tui()`'s original code was:

  ```rust
  enable_raw_mode()?;
  execute!(stdout, EnterAlternateScreen)?;
  let result = draw_tui(&mut terminal, workspaces); // if this panics...
  disable_raw_mode()?;                               // ...these two lines never run
  execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
  ```

  If `draw_tui` panics internally — even from something as mundane as an
  off-by-one index bug — `disable_raw_mode` and "leave the alternate
  screen" never execute, and the user's terminal is left stuck in raw
  mode plus the alternate screen: Enter does nothing, `Ctrl-C` may not
  even work. Miserable. The fix is to wrap the render loop in
  `std::panic::catch_unwind`, restore terminal state, and only then
  re-raise the panic:

  ```rust
  let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      draw_tui(&mut terminal, workspaces)
  }));
  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
  match result {
      Ok(result) => result,
      Err(payload) => std::panic::resume_unwind(payload),
  }
  ```

  lazygit has the equivalent consideration: `onWorkerAux`
  (`pkg/gocui/gui.go`) wraps every background goroutine in panic recovery,
  and an unrecovered panic first calls `Screen.Fini()` (tcell's cleanup
  function for leaving the alternate screen and restoring terminal mode)
  before re-panicking. **The principle is identical: any path that might
  panic, once it happens after the terminal has been switched into a
  special mode, must explicitly restore the terminal — you cannot rely on
  the normal execution path to do it as a side effect.** This lesson
  generalizes to nearly every language/framework that touches raw terminal
  mode directly (Node's blessed/ink and Python's curses both have
  equivalent `finally`/`atexit` patterns).

---

## 8. Side-by-Side Comparison

| Dimension | zellij-workbench (ratatui) | lazygit (gocui/tcell) |
|---|---|---|
| Architecture | Immediate mode: rebuild the widget tree every frame | Persistent `View` objects + dirty-flag partial rebuilds |
| Event loop | Single-threaded `loop`, `event::poll(200ms)` | Single-threaded `processEvent`, two channels (input / user callbacks) merged |
| Render diffing | `ratatui-core::BufferDiff` (cell-level) | tcell `CellBuffer.Dirty()` (cell-level) + gocui's own `tainted`/content-only fast path (view-level) |
| Long-list optimization | No virtualization (not needed at current data scale) | `RenderOnlyVisibleLines`: formats only rows inside the visible window |
| Layout system | `Layout` + `Constraint` (Length/Percentage/Min) | Custom `boxlayout`: a `Size`/`Weight` recursive tree, flexbox-style algorithm |
| Navigation/modes | Flat `InputMode { Normal, Search }` enum | `ContextMgr` context stack, push/pop rules driven by `ContextKind` |
| Async | `thread::spawn` + `mpsc::channel`, main loop uses non-blocking `try_recv` | `OnWorker` goroutine + `OnUIThread`/`Update` handback, per-scope locks to prevent overlapping refreshes |
| Throttling/caching | None yet (scan frequency and data volume don't require it) | Commit-graph cache, colored-string cache, git-config cache, command-start throttling (30ms) |
| Keybinding discovery | Fixed bottom hint line | Bottom options bar + full `?` keybinding menu, both sharing the same binding data |
| Terminal-state recovery | `catch_unwind` wrapping the render loop (this guide's fix) | Background-goroutine panic recovery + `Screen.Fini()` |

**How to read this table**: it's not "lazygit is better everywhere" —
it's that each project's complexity investment matches the scale of its
own problem. lazygit has to handle real Git repositories of any size
(potentially hundreds of thousands of commits) and complex multi-panel
navigation across several screen modes, so it has done real work on
virtualization, caching, throttling, and the context stack.
`zellij-workbench`'s current data scale (tens to low hundreds of
workspaces) and interaction complexity (list + detail + search) are
entirely well served by immediate mode plus flat state. **Keep
architectural complexity at "just enough" for now, and only reach for
virtualization, caching, or a context stack — following this guide's
lead — once you hit an actual signal that "this operation is visibly
slow" or "mode management has become unmanageable." Don't pre-pay
complexity for a requirement that hasn't shown up yet.**

---

## 9. Further Reading

- [Ratatui: Rendering under the hood](https://ratatui.rs/concepts/rendering/under-the-hood/) —
  the official docs' detailed explanation of the `Buffer` diffing
  mechanism.
- [Ratatui ARCHITECTURE.md](https://github.com/ratatui/ratatui/blob/main/ARCHITECTURE.md) —
  the multi-crate split (`ratatui-core`/`ratatui-widgets`/various backends)
  since 0.30.
- [The Elm Architecture (bubbletea's take)](https://github.com/charmbracelet/bubbletea) —
  the Model-Update-View pattern realized in Go, with an official tutorial.
- [lazygit's source](https://github.com/jesseduffield/lazygit), especially
  `pkg/gocui/gui.go` (event loop), `pkg/gui/context.go` (context stack),
  `pkg/tasks/tasks.go` (streaming command output and throttling), and
  `vendor/.../lazycore/pkg/boxlayout` (the layout algorithm).
- This repo's own `src/tui.rs` — nearly every ratatui example in this
  guide is lifted directly from that file; reading it alongside this guide
  is worthwhile.

If you're starting a TUI from scratch, a reasonable order is: get the main
flow working with immediate mode plus a flat state enum (the "starter kit"
from sections 1 and 5), then wire up the event loop and async refresh
skeleton per section 2, and only then — as you actually run into
performance or navigation complexity — gradually "add ingredients"
following lazygit's approach in sections 3, 4, and 5. Don't front-load a
context stack, virtualized lists, and throttling — mechanisms meant for
large-scale scenarios — from day one.
