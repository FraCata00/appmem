# appmem

See which macOS app is using your memory.

Activity Monitor lists dozens of scattered processes (Chrome helpers,
renderers, SourceKit, simulator services…) and never tells you the total per
app. `appmem` groups every process under the app it belongs to and shows the
totals in your terminal, heaviest first. Expand an app to see its biggest
processes, and quit the app or terminate a process without leaving the
keyboard.

```
Memory 6.90/8.00 GB (wired 1.40)  Pressure ■■■■■□□□ warn  Swap 2.10 GB
▾ Google Chrome          42 proc    3.12 GB  █████░░░░░░░░
    Google Chrome Helper (Renderer)     812     820 MB
    Google Chrome Helper (Renderer)     907     610 MB
    Google Chrome Helper (GPU)          401     402 MB
    … 39 more                                  1.29 GB
▸ Xcode                  11 proc    2.41 GB  ████░░░░░░░░░
▸ Simulator               6 proc     901 MB  █░░░░░░░░░░░░
▸ System                412 proc   ~1.10 GB  ██░░░░░░░░░░░
↑↓ move  ⏎ expand/collapse  k terminate  q quit
```

It is small on purpose: about 4 MB of memory and no measurable CPU while
idle, so it doesn't make things worse on a Mac that is already short on RAM.

## Install

Requires macOS 12 or later.

### Prebuilt binary (Apple Silicon)

Download the latest build from the
[Releases page](https://github.com/FraCata00/appmem/releases), or from the
terminal:

```sh
curl -LO https://github.com/FraCata00/appmem/releases/download/v0.1.0/appmem-v0.1.0-aarch64-apple-darwin.tar.gz
tar -xzf appmem-v0.1.0-aarch64-apple-darwin.tar.gz
sudo mv appmem-v0.1.0-aarch64-apple-darwin/appmem /usr/local/bin/
```

Each release also has a `.sha256` file; check the download with
`shasum -a 256 -c appmem-v0.1.0-aarch64-apple-darwin.tar.gz.sha256`.

The binary is not notarized. Downloading it with `curl`, as above, avoids
Gatekeeper's quarantine flag. If you download it with a browser instead, run
`xattr -d com.apple.quarantine appmem` before the first launch.

### With Cargo

On Intel Macs, or to build from source, use a Rust toolchain:

```sh
cargo install appmem
```

## Usage

```sh
appmem                 # interactive view, refreshes every 2 seconds
appmem --interval 5    # refresh every 5 seconds (minimum 0.5)
appmem --once          # print the per-app table once and exit
```

| Key | Action |
| --- | --- |
| ↑ ↓ | Move |
| Enter | Expand or collapse an app |
| k | On an app: quit it (like Cmd-Q). On a process: terminate it (SIGTERM). Always asks first. |
| q, Esc, Ctrl-C | Exit |

Set `NO_COLOR=1` to turn colors off.

## How memory is measured

- **Per process**: the physical footprint (`ri_phys_footprint`), the same value
  Activity Monitor shows in its Memory column. It includes compressed memory.
- **Per app**: the sum of its processes. Shared memory is counted once per
  process, exactly as when you add up Activity Monitor's numbers by hand.
- **Header**: used memory is computed like Activity Monitor's "Memory Used"
  (app memory + wired + compressed). Wired memory belongs to the kernel and
  drivers, so it is shown in the header rather than as a row.
- **System** (always last): Apple's background processes, plus an estimate
  (marked `~`) for memory used by processes appmem can't read without `sudo`.

## How processes are grouped

1. Simulator runtime processes and Simulator.app go to **Simulator**, shown
   separately from Xcode.
2. A process belongs to the app macOS holds *responsible* for it. This is
   how Safari's WebKit processes or `rust-analyzer` started by your editor end
   up under the right app.
3. Anything else under `/System`, `/usr`, `/bin`, `/sbin` or `/Library/Apple`
   goes to **System**, except Apple's own apps (Safari, Notes, Mail…).
4. Otherwise, the outermost `.app` bundle in the executable path.
5. Your other processes go to **Command-line tools**; the rest to System.

Apps that only have widgets or helpers running (Weather, Clock…) appear with
those processes. They can't be quit as a whole, since asking a closed app to
quit would launch it first.

## Safety

- System processes, processes of other users, and appmem itself with the
  shell and terminal running it are never terminated.
- Quitting an app sends a normal quit request, so apps can ask you to save
  unsaved work. The first time, macOS asks to allow your terminal to control
  that app (System Settings → Privacy & Security → Automation).
- appmem never sends SIGKILL.

## Chrome tabs

From outside Chrome there is no way to know which tab a renderer process
belongs to, so appmem shows renderers by name and PID. To find a heavy tab,
use Chrome's own Task Manager (Shift+Esc).

## License

MIT
