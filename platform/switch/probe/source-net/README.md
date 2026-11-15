# Java Server STATUS Probe

Native Nintendo Switch probe for Minecraft: Java Edition server STATUS ping (no login/gameplay).

## Interface

Implements `np_run(struct np_job *)` from `platform/switch/net-probe/include/net_probe.h`.

## Files

- **transport.c**: POSIX C socket transport (IPv4 TCP, nonblocking I/O, 10s deadline after DNS, cancellation slices ≤100ms). No libnx/switch.h/UI/threading: enables macOS socket fixtures for logic validation.
- **main.c**: libnx console/pad/swkb UI with static job and native Thread. Sockets initialized once; all failures visible with release semantics.

## UI Controls

- **X**: Edit server host (keyboard)
- **Y**: Edit port (keyboard, 1-65535)
- **A**: Connect explicitly (disabled while busy or if host empty)
- **B**: Cancel active connection
- **Plus**: Exit

Edits and connect disabled during active connection. Thread joined before reuse/exit.

## Display

- Stage progression (IDLE, RESOLVING, CONNECTING, SENDING, RECEIVING, DONE, ERROR, CANCELLED)
- Error details: type, system errno, resolver error, codec error
- Response length, JSON byte range `[offset, offset+length)`
- Bounded sanitized raw JSON (control chars/non-ASCII replaced with `?`, truncated at 800 chars)

No MOTD parsing, no server-controlled format strings/ANSI. Tests framing and native sockets only.

## Implementation Details

### Transport (POSIX C)

- **DNS**: IPv4 `getaddrinfo(AF_INET, SOCK_STREAM)`. May block; cancellation waits for it to complete (no safe interruption).
- **Connect**: Nonblocking with `select()` timeout and SO_ERROR check.
- **I/O**: Partial send/recv loops, cancellation checked before operations (≤100ms granularity via `select()` slices).
- **Deadline**: Monotonic 10 seconds AFTER DNS resolution (`CLOCK_MONOTONIC`).
- **Framing**: Rust codec (`craftrustnx_status_encode`, `craftrustnx_status_decode`) for handshake/request and response parsing.
- **Cleanup**: Sockets and address lists closed/freed before publishing terminal stage with release semantics.

### Main (libnx)

- **Initialization**: `socketInitializeDefault()` once at startup; failure displayed prominently.
- **Job**: Static struct with atomic `cancel` and `stage` fields. Main writes cancel/reads stage atomically; only reads result fields after acquiring a terminal stage.
- **Thread**: libnx `Thread` for `np_run()`. Joined and closed before reuse or exit.
- **Keyboard**: `swkbd` for host/port editing (disabled while thread active).
- **Localization**: Diagnostic labels centralized in string constants for later translation.

## Build and verification

See [net-probe](../../net-probe/README.md) for the complete offline NRO build,
loopback host fixture commands, and current limitations. Build from the repository
root with `bash scripts/build-switch.sh docker net`.

The main loop checks thread creation/start failures, reaps workers before reuse,
and cancels/joins on applet exit. Plus requests cancellation while continuing to
draw the UI until the worker finishes; blocking DNS can delay this exit.

The transport attempts only the first resolved IPv4 address. Host fixtures test
real C sockets linked to the Rust codec; they do not execute Switch code.
