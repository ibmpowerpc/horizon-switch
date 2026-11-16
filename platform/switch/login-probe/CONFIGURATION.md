# Configuration / first play packet probe

Build offline using the existing SDK image:

```sh
bash scripts/build-switch.sh docker configuration
```

Output: `dist/switch/craftrustnx/craftrustnx-configuration-probe.nro`, homebrew
title **CraftRustNX Configuration Probe**, version **0.3.0**. This separate target
preserves the hardware-tested encrypted-login probe. It uses the same offline
profile and RSA/AES negotiation, without sending a session-service join. A
server may accept or refuse the attempt under its own authentication rules.

Run through hbmenu in Atmosphere. X edits host, Y port, L player name, A starts,
B cancels, Plus exits after cleanup. Use the current Java LAN port; it changes
when the world is reopened. The default CraftRustNX is the player name, not the
server's name. A duplicate player name can cause a server refusal.

Expected success: `Stage: PLAY_PACKET_SEEN`, `Error: NONE`, `Configuration ACK:
yes` and a first play packet ID/body length. This stops and closes the connection
immediately. It does not load registries, render a world, or let you play.

The probe responds to keepalive/ping, claims no known packs, reports registry
packet counts and accepts their bodies opaquely. It declines resource packs
and stops on code-of-conduct text instead of accepting it automatically. A server
requiring a pack can disconnect normally. Transfers stop without reconnecting.
A USER_ACTION_REQUIRED or UNSUPPORTED result is distinct from successful phase
completion. Disconnect text is raw sanitized component data, not a parsed message.

Runtime bounds: approximately 10 MiB additional heap, 128 KiB worker stack,
2 MiB framed / 8 MiB decoded body, 4096 packets and 64 MiB decoded per attempt.
The 30-second deadline begins after blocking IPv4 DNS. Cancellation checks socket
waits at intervals no longer than 100 ms, but cannot interrupt DNS or a single
bounded inflate operation. Every terminal stage follows resource cleanup.

Source contract and limitations: [configuration audit](../../../docs/switch-configuration-audit.md).
For a hardware result, record the Stage/Error, configuration ACK, first play
packet ID/length, registry count, and whether encryption is active. Do not infer
a successful game connection from the title or an earlier login acceptance.
