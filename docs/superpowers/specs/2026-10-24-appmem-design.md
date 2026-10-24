# appmem — design

Date: 2026-10-24
Status: draft, awaiting approval

## Intent

`appmem` is a terminal tool for macOS that answers one question, fast:
**which app is eating my memory, in total?**

The author runs an 8 GB M1 Mac that is under chronic memory pressure. The
moment that motivated this: Chrome and Xcode open together, the machine slows
down, and Activity Monitor shows dozens of scattered processes (Chrome
helpers, renderers, SourceKit, simulator services…) with no per-app total. It
is impossible to tell at a glance who is responsible.

Once the heavy app is identified, the author wants to act on it: close the
heaviest processes (e.g. a Chrome renderer) right from the tool.

It is meant to be published as an open-source project, so the code should be
clean and approachable for contributors.

## Goals

1. Show per-app memory totals, with every process attributed to the app it
   belongs to.
2. Show system-wide state in one line: memory used / total, memory pressure
   level, swap used.
3. Let the user expand an app to see its heaviest processes.
4. Let the user terminate a process from the tool, with confirmation.
5. Numbers must match what Activity Monitor shows, so the user trusts them.
6. Stay tiny: a tool for a memory-starved machine must not use much memory
   itself (target: under ~15 MB resident, negligible CPU at idle refresh).

## Non-goals (v1)

- History or graphs of memory pressure over time.
- Mapping Chrome renderer processes to tab names. From outside Chrome this
  information is not available; the tool will label them by process name
  (e.g. "Google Chrome Helper (Renderer)") and PID. The README should point
  users to Chrome's own Task Manager (Shift+Esc) for tab-level detail.
- Linux or Windows support. macOS only.
- A GUI, menu-bar app, or daemon.
- CPU, disk, or network monitoring.

## Anti-goals

- Numbers that disagree with Activity Monitor. If the user sees "Chrome
  3.1 GB" here and something wildly different there, the tool has failed even
  if it technically works.
- Letting the user kill something that destabilizes the system (kernel_task,
  WindowServer, launchd, other users' or root processes).
- Requiring `sudo` to be useful.
- Being heavy or slow itself.

## The screen (chosen layout: tree list)

The user picked a tree layout from three mockups. Sketch (numbers invented):

```
Memory  6.9/8.0 GB   Pressure ■■■■■■□□ warn   Swap 2.1 GB

▾ Google Chrome        42 proc   3.12 GB  ███████████░░
    Google Chrome Helper (Renderer)  812    820 MB
    Google Chrome Helper (Renderer)  907    610 MB
    Google Chrome Helper (GPU)       401    402 MB
    … 39 more                               1.29 GB
▸ Xcode                 11 proc   2.41 GB  ████████░░░░░
▸ Simulator              6 proc   0.88 GB  ███░░░░░░░░░░
▸ System                          0.52 GB  ██░░░░░░░░░░░

↑↓ move  ⏎ expand/collapse  k terminate  q quit
```

Behavior:

- Header line: used / total physical memory, pressure level (normal / warn /
  critical, colored green / yellow / red), swap used.
- One row per app, sorted by total memory, descending. Each row: expand
  marker, app name, process count, total memory, a bar proportional to total
  physical memory.
- Expanding an app shows its processes sorted by memory, descending. Show the
  top N (builder's call, ~5–8) and collapse the rest into "… N more" with
  their combined total.
- Selection (highlighted row) stays on the same app/process across refreshes
  even when sort order changes; if the selected process exits, selection moves
  to its app row.
- Auto-refresh every 2 seconds by default (`--interval` flag to change it).
- Keys: ↑/↓ move, Enter expand/collapse, `k` terminate, `q` / Esc / Ctrl-C
  quit. (No vim-style j/k navigation, since `k` is terminate.)
- Terminate (`k`), decided by Claude after review:
  - **On a process row**: asks "Terminate Google Chrome Helper (Renderer)
    (pid 812, 820 MB)? y/N" and sends SIGTERM to that one PID.
  - **On an app row**: asks "Quit Google Chrome? y/N" and sends a normal
    quit request, the same as Cmd-Q, to the app (Apple Event via
    `osascript -e 'tell application id "<bundle id>" to quit'`, or an
    equivalent native call). Never signals all its processes at once: the app
    shuts down its own helpers and gets the chance to ask about unsaved work
    (important for Xcode). The first use triggers macOS's Automation
    permission prompt; the README must mention this. The "Command-line tools"
    group has no app-level quit.
  - **Refused**, with a message in the status line: processes not owned by the
    current user, anything in the "System" group, `appmem` itself and its
    parent shell / terminal.
  - SIGTERM only, never SIGKILL in v1.
- Must work in a standard 80×24 terminal; uses more width if available.
- Respects `NO_COLOR`.

## How memory is measured

Decided (author delegated these calls):

- **Per-process memory = physical footprint** (`ri_phys_footprint` from
  `proc_pid_rusage` with `RUSAGE_INFO_V4` or later). This is the value
  Activity Monitor shows in its "Memory" column; it includes compressed
  memory attributable to the process.
- **App total** = sum of the physical footprint of its processes. (Note: this
  double-counts shared memory slightly, exactly as summing in Activity Monitor
  would. Acceptable; it's consistent with what users see.)
- **Used / total**: total from `hw.memsize`; "used" computed from
  `host_statistics64` (`HOST_VM_INFO64`) the way Activity Monitor computes
  "Memory Used" (app memory + wired + compressed). The builder should verify
  the formula against Activity Monitor on a real machine.
- **Pressure level**: `sysctl kern.memorystatus_vm_pressure_level`
  (1 = normal, 2 = warn, 4 = critical).
- **Swap**: `sysctl vm.swapusage`.

Processes owned by root or other users usually cannot be read without
privileges. Decided by Claude after review:

- **Wired memory** (kernel and drivers) is shown in the header line, e.g.
  `Memory 6.9/8.0 GB (wired 1.4)`, not as a row: the user can't act on it.
- The **"System" row** contains readable system processes. Only when some
  processes can't be read, it also gets an estimated remainder, labeled with
  `~`: app memory counted uncompressed (internal − purgeable pages plus
  `total_uncompressed_pages_in_compressor`) minus every listed footprint.
  Footprints count compressed pages at full size, so they are compared in
  that space, not with "used".
- "System" is **always pinned to the bottom** of the list regardless of size,
  so the top of the list always answers "which app can I do something
  about".
- Rows do **not** add up to "used": footprints count compressed memory at its
  uncompressed size and shared memory once per process, as Activity Monitor
  does. Per-process numbers are what must match Activity Monitor.

## How processes are grouped into apps

Decided: a process belongs to the **outermost `.app` bundle in its executable
path**. Example: `/Applications/Google Chrome.app/Contents/Frameworks/…/Google
Chrome Helper (Renderer).app/Contents/MacOS/…` → "Google Chrome". Executable
path from `proc_pidpath`. App display name: the bundle's `CFBundleName` /
`CFBundleDisplayName` from `Info.plist`, falling back to the folder name
without `.app`.

Attribution rules, applied in this order (decided by Claude after review;
the author delegated these calls):

1. **Simulator runtimes → "Simulator".** Any process whose path contains
   `.simruntime/` or `/CoreSimulator/`, plus `Simulator.app` itself, goes to a
   single "Simulator" row. Simulator is shown **separately from Xcode** (as in
   the mockup) even though `Simulator.app` lives inside `Xcode.app`: the user
   wants to see what the simulator costs on its own.
2. **Responsible process, if available.** Use
   `responsibility_get_pid_responsible_for_pid` (private libsystem API, looked
   up at runtime with `dlsym`; if missing, skip this step silently) and
   attribute the process to the app of its responsible process, when that is
   an app or the Simulator. This is what Activity Monitor does and catches
   XPC services launched by Xcode, Safari's WebKit processes, etc. System
   daemons such as loginwindow or WindowServer are responsible for themselves,
   so they never land in an app this way.
3. **Apple system paths → "System".** Any process whose executable lives
   under `/System/`, `/usr/`, `/bin/`, `/sbin/`, or `/Library/Apple/` goes to
   "System" (Finder, Dock, loginwindow, ControlCenter, Spotlight…). This keeps
   the list short and keeps dangerous processes out of reach of terminate.
   **Exception:** Apple's own apps in `/System/Applications/` and
   `/System/Volumes/Preboot/Cryptexes/App/System/Applications/` (Safari,
   Notes, Mail…) are apps like any other and follow rule 4.
4. **Outermost `.app` in the executable path** (fallback).
5. **Anything else owned by the current user** (shell, CLI tools, `cargo`,
   `rust-analyzer` launched outside an app…) goes to a single
   "Command-line tools" row, expandable like any app.
6. **Anything else** → "System".

Unit tests for these rules must cover at least: a Chrome renderer, a
`/System/Library/CoreServices/Finder.app` process, a Simulator `.simruntime`
process, `Xcode.app/Contents/Developer/Applications/Simulator.app`, and a
plain user CLI binary.

## Constraints

- macOS on Apple Silicon is the primary target (author's machine: M1, 8 GB);
  Intel Macs should work if nothing special is needed. Minimum macOS version:
  builder's call, document it.
- Rust, stable toolchain. Rust is **not installed** on the author's machine
  yet; the plan's first step is installing it via `rustup`.
- Compile footprint matters on 8 GB: keep dependencies few and light. No
  async runtime.
- No `sudo` needed for normal use.
- Code, comments, commit messages, README in English. Commit messages follow
  Conventional Commits.

## Decided "how"

- Language: Rust.
- TUI: `ratatui` + `crossterm`.
- macOS data: via `libc` / `libproc` FFI (or a small, well-maintained crate
  such as `libproc` if it exposes `proc_pid_rusage` v4+); the builder chooses,
  favoring fewer dependencies.
- Distribution: `cargo install appmem` (crates.io name `appmem` was free on
  2026-10-24). Homebrew tap later, not in v1.
- License: MIT.
- Project location: `~/Projects/appmem`, a git repo.

## Left to the builder

- Internal architecture, module layout. Suggested split: a data-collection
  layer (macOS-specific, behind a small trait so grouping/sorting logic can be
  unit-tested with fake process lists) and a UI layer.
- How many processes to show when expanded before "… N more".
- Exact colors, bar style, number formatting (GB with two decimals above
  1 GB, MB below seems reasonable).
- Whether to offer a non-interactive mode (e.g. `appmem --once` printing the
  table and exiting). Nice for testing and for the README; optional.
- Test strategy beyond unit tests for grouping/sorting.
- README content, CI setup (GitHub Actions on a macOS runner is a sensible
  default).

## Success looks like

With Chrome and Xcode open on the author's Mac, running `appmem` shows within
a second or two a short list where Chrome and Xcode are near the top with
totals that match Activity Monitor's numbers (summed by hand), and the author
can expand Chrome and terminate its heaviest renderer in a couple of
keystrokes.
