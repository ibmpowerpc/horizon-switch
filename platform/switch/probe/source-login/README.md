# source-login: Native Switch Java Server LOGIN Probe

Diagnostic probe for Minecraft Java Edition 26.3 offline login handshake. Stops
at LOGIN_ACCEPTED (validated LoginFinished) BEFORE configuration or play phases.

## Purpose

Hardware validation of the login protocol codec (Rust FFI), socket transport,
and UI responsiveness under real network conditions. No encryption (offline
only), no configuration packets, no gameplay.

## Implementation

- **main.c**: libnx UI with console, pad, swkbd, and worker thread
- **transport.c**: POSIX C sockets (IPv4 only, no switch.h)
- **Headers**: normative `login_probe.h` and `login_codec.h` from
  `platform/switch/login-probe/include/`

## Protocol Behavior

1. Handshake (next state = login) + LoginStart with offline UUID v3
2. Receive login response frames with optional compression
3. Terminal outcomes:
   - **LOGIN_ACCEPTED**: LoginFinished validated (no LoginAcknowledged sent)
   - **DISCONNECTED**: server sent Disconnect packet
   - **AUTH_OR_ENCRYPTION_REQUIRED**: EncryptionRequest received (unsupported;
     even should_authenticate=false requires unsupported encryption)
   - **UNSUPPORTED**: CustomQuery or CookieRequest (not implemented)
   - **ERROR**: network, protocol, or codec error
   - **CANCELLED**: user cancelled (B button)

Offline vanilla servers normally SKIP EncryptionRequest. Online-mode authentication is not implemented.

## Compression Handling

- Initial threshold: -1 (uncompressed framing)
- SetCompression packet updates threshold and compression state
- Buffered frames are parsed immediately after compression change (coalescing)
- Codec validates compression envelope and zlib body per Java 26.3 rules

## Constraints

- Max 16 events before protocol error (bounded CPU)
- Max body 65536 bytes, max wire 65541 bytes (header CAP rejects oversized packets)
- 10-second deadline AFTER DNS resolution (DNS may block cancellation)
- 100ms cancellation granularity during socket I/O
- FD_SETSIZE guard for select()
- No 8MB scratch buffer, no watchdog

## Controls

- **X**: Edit host (hostname or IP)
- **Y**: Edit port (default 25565)
- **L**: Edit player name (1-16 ASCII letters/digits/underscore, default CraftRustNX)
- **A**: Connect (disabled if host empty or sockets failed)
- **B**: Cancel (only while active)
- **Plus**: Exit (cancels worker, waits for join)

## Display

- Stage progression (IDLE → RESOLVING → CONNECTING → SENDING → RECEIVING → terminal)
- Compression threshold and frame count
- LOGIN_ACCEPTED: server name, server UUID, session UUID
- DISCONNECTED/UNSUPPORTED: bounded sanitized event payload (600 chars)
- Errors: system errno, DNS error code, codec error code
- DNS limitation visible, no crash on code failure

## Build and verification

From the repository root: `bash scripts/build-switch.sh docker login`.
Uses the existing image with `--pull=never --network none`; builds installed Rust
sources offline. See [login-probe](../../login-probe/README.md) for the full
host fixture command and scope. No game assets are required.

Both packet buffers are in the static job. The worker stack is 128 KiB for
Rust/zlib calls; C build warns on frames larger than 32 KiB. Compile-only objects
must stay outside Makefile's build-login directory so non-PIE objects cannot be
reused in the final link.

## Differences from source-net (STATUS probe)

- Two-frame request (handshake + LoginStart) vs handshake + status request
- Multi-frame response with compression vs single STATUS response
- Terminal outcomes: ACCEPTED/DISCONNECTED/AUTH_REQUIRED/UNSUPPORTED vs DONE
- Player name input (L button)
- Session UUID display
- No MOTD/JSON parsing (LOGIN has structured events)

## Audit Notes

- Offline vanilla normally skips EncryptionRequest (not "empty keys false")
- Cookies/queries are not proxy-only; they are valid login-phase packets
- Distinct default name (CraftRustNX) avoids disrupting host player's profile
- Header CAP (65541 bytes) deliberately rejects over-64K unsupported packets
- No 8MB scratch or watchdog: bounded CPU by design
- `atomic_init` and static direct init used (GCC 15 gnu23 default)
