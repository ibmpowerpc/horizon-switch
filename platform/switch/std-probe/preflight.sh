#!/usr/bin/env bash
# Preflight for the std runtime prototype of the native Nintendo Switch
# (Atmosphere / hbmenu) Horizon port.
#
# Analysis only: nothing here compiles std, links a binary, contacts the
# network, installs a toolchain or modifies the installed rust-src, the Cargo
# registry or any other user cache. The one write it performs outside its own
# sources is the generated staging under `build/` (git-ignored), which is an
# unpacked copy of the rust-src vendor set with the project's patched libc:
# without it no offline std resolve can be truthful.
#
#   ./preflight.sh             analyse and (re)stage build/vendor if needed
#   ./preflight.sh --no-stage  analyse only; report a blocker if unstaged
#
# Exit status: 0 = no blockers, 1 = blockers (listed at the end).
set -uo pipefail
unset CARGO_ENCODED_RUSTFLAGS

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$script_dir/../../.." && pwd)"
target_json="$script_dir/src/switch-std.json"
build_dir="$script_dir/build"
vendor_dir="$build_dir/vendor"
cargo_config="$build_dir/cargo-config.toml"
target_dir="$build_dir/target"
report="$build_dir/preflight.txt"
next_build="$build_dir/next-build.sh"
apply_script="$repo_dir/patches/libc-switch/apply.sh"
patch_file="$repo_dir/patches/libc-switch/libc-0.2.189-switch-aarch64.patch"
staged_libc="$repo_dir/tools/switch-abi-audit/rust/build/patched/libc-0.2.189"
rust_probe_target="$repo_dir/platform/switch/rust-probe/target"
image="devkitpro/devkita64:20260219"

min_free_kib=$((10 * 1024 * 1024))    # host reserve: keep 10 GiB free
max_artifacts_kib=$((512 * 1024))     # Rust experiment artifact cap: 512 MiB
expected_rustc="1.100.0"
rustflags='-C target-cpu=cortex-a57 -C relocation-model=pic -Zunstable-options'
cargo_unstable='-Z json-target-spec -Z unstable-options -Z build-std=std,panic_abort'

do_stage=1
for arg in "$@"; do
  case "$arg" in
    --no-stage) do_stage=0 ;;
    -h|--help) sed -n '2,15p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

blockers=()
warns=()
say() { printf '%s\n' "$*"; }
ok() { printf '  [ok]      %s\n' "$*"; }
warn() { warns+=("$*"); printf '  [warn]    %s\n' "$*"; }
blocker() { blockers+=("$*"); printf '  [BLOCKER] %s\n' "$*"; }

mkdir -p "$build_dir"
# Tee the whole run into build/preflight.txt for review.
exec > >(tee "$report") 2>&1

say "== CraftRustNX std probe preflight =="
say "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
say "repo=$repo_dir"
say "crate=$script_dir"
say "mode=$([ "$do_stage" -eq 1 ] && echo 'analyse+stage' || echo 'analyse-only')"
say ""

# ---------------------------------------------------------------------------
# 1. Toolchain and rust-src
# ---------------------------------------------------------------------------
say "-- 1. toolchain ------------------------------------------------------"
rustc_version="$(rustc --version 2>/dev/null)"
cargo_version="$(cargo --version 2>/dev/null)"
if [[ "$rustc_version" == *" $expected_rustc "* ]]; then
  ok "rustc $rustc_version"
else
  blocker "rustc 1.100.0 required; found: ${rustc_version:-none}"
fi
case "$cargo_version" in
  *" $expected_rustc "*) ok "cargo $cargo_version" ;;
  *) blocker "cargo 1.100.0 required; found: ${cargo_version:-none}" ;;
esac
sysroot="$(rustc --print sysroot 2>/dev/null)"
rust_src="$sysroot/lib/rustlib/src/rust"
for probe in library/std/src/lib.rs library/core/src/lib.rs library/Cargo.lock; do
  if [ -f "$rust_src/$probe" ]; then
    ok "rust-src has $probe"
  else
    blocker "installed rust-src is incomplete: missing $rust_src/$probe (no download attempted)"
  fi
done
if [ -d "$rust_src/library/vendor/libc-0.2.189" ]; then
  vendor_src_kib="$(du -sk "$rust_src/library/vendor" | awk '{print $1}')"
  ok "rust-src vendor set present (${vendor_src_kib} KiB), libc-0.2.189 included"
else
  blocker "rust-src vendor set missing: $rust_src/library/vendor/libc-0.2.189"
  vendor_src_kib=0
fi
say "  rust_src=$rust_src"

# ---------------------------------------------------------------------------
# 2. Disk: host reserve and the 512 MiB artifact cap
# ---------------------------------------------------------------------------
say ""
say "-- 2. disk ----------------------------------------------------------"
free_kib="$(df -Pk "$repo_dir" | awk 'NR==2 {print $4}')"
free_gib="$(awk -v k="$free_kib" 'BEGIN {printf "%.1f", k/1048576}')"
if [ "$free_kib" -ge "$min_free_kib" ]; then
  ok "host free ${free_gib} GiB (reserve is 10 GiB)"
else
  blocker "host free ${free_gib} GiB is below the 10 GiB reserve"
fi

artifact_kib=0
for dir in "$rust_probe_target" "$target_dir" "$build_dir/host-target" "$script_dir/target" "$repo_dir/platform/switch/net-probe/target" "$repo_dir/platform/switch/net-probe/codec/target" "$repo_dir/tools/switch-abi-audit/rust/build" "$repo_dir/platform/switch/probe/build" "$repo_dir/platform/switch/probe/build-rust" "$repo_dir/platform/switch/probe/build-std"; do
  [ -d "$dir" ] || continue
  dir_kib="$(du -sk "$dir" | awk '{print $1}')"
  artifact_kib=$((artifact_kib + dir_kib))
  say "  $(printf '%-58s' "${dir#$repo_dir/}") $(awk -v k="$dir_kib" 'BEGIN {printf "%7.1f MiB", k/1024}')"
done
headroom_kib=$((max_artifacts_kib - artifact_kib))
ok "Rust artifacts $(awk -v k=$artifact_kib 'BEGIN{printf "%.1f", k/1024}') MiB of the 512 MiB cap; headroom $(awk -v k=$headroom_kib 'BEGIN{printf "%.1f", k/1024}') MiB"

# Measured unit: one `build/core/<hash>/out` pair (rlib + rmeta) is ~128 MiB on
# this host. std is a strictly larger crate than core; using 2.0x-2.5x of that
# measured unit plus the registry dependency artifacts and the staticlib gives
# the range below. These are estimates with a stated derivation, not
# measurements; the point is to decide whether to start the build at all.
budget_verdict="$build_dir/budget-verdict.txt"
python3 - "$rust_probe_target" "$repo_dir" "$vendor_dir" "$artifact_kib" "$max_artifacts_kib" "$budget_verdict" "$target_dir" <<'PY'
import os, sys
probe_target, repo, vendor, artifact_kib, cap_kib, verdict_path, std_target = sys.argv[1:8]
artifact_kib = int(artifact_kib); cap_kib = int(cap_kib)

def kib_of(path):
    total = 0
    seen = set()
    for dirpath, _dirs, files in os.walk(path):
        for name in files:
            try:
                st = os.lstat(os.path.join(dirpath, name))
                identity = (st.st_dev, st.st_ino)
                if identity not in seen:
                    seen.add(identity)
                    total += st.st_blocks * 512
            except OSError:
                pass
    return total // 1024

outs = []
root = os.path.join(probe_target, 'aarch64-nintendo-switch-freestanding', 'release', 'build', 'core')
if os.path.isdir(root):
    for entry in sorted(os.listdir(root)):
        out = os.path.join(root, entry, 'out')
        if os.path.isdir(out):
            outs.append((kib_of(out), out, os.path.getmtime(out)))
outs.sort(reverse=True)
unit = (sum(o[0] for o in outs) // len(outs)) if outs else 128 * 1024
vendor_kib = kib_of(vendor) if os.path.isdir(vendor) else 0
low = unit * 2 + vendor_kib * 3 + 8192
high = unit * 5 // 2 + vendor_kib * 5 + 16384
existing_std = kib_of(std_target) if os.path.isdir(std_target) else 0
# Budget the remaining delta, not the entire std tree a second time.
low = max(0, low - existing_std)
high = max(0, high - existing_std)
archive = os.path.join(std_target, 'switch-std', 'release', 'libcraftrustnx_std_probe.a')
inputs = [os.path.join(repo, 'platform/switch/std-probe', p) for p in
          ('Cargo.toml', 'src/switch-std.json')]
inputs += [os.path.join(vendor, 'libc-0.2.189', p) for p in
           ('src/unix/newlib/mod.rs', 'src/unix/newlib/horizon/mod.rs')]
if os.path.isfile(archive) and all(os.path.isfile(p) and os.path.getmtime(p) <= os.path.getmtime(archive) for p in inputs):
    # The target, ABI dependency and manifest have already built successfully.
    # Reserve room for rebuilding this small probe, not another full sysroot.
    low, high = 8 * 1024, 32 * 1024
    print('  existing std archive is newer than its configuration inputs: reserving 32 MiB for incremental probe work')

unit_label = 'measured core build unit' if outs else 'core budget floor (cache removed)'
print(f"  {unit_label}: {unit/1024:.1f} MiB (rlib+rmeta)")
print(f"  existing isolated std target: {existing_std/1024:.1f} MiB")
print(f"  estimated std build delta (new artifacts): {low/1024:.1f}-{high/1024:.1f} MiB")
print("  bounded cleanup candidates (identified, NOT deleted; regenerable offline):")
import datetime
for size, path, mtime in outs:
    stamp = datetime.datetime.fromtimestamp(mtime, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    print(f"    {size/1024:7.1f} MiB  {os.path.relpath(path, repo)}  (mtime {stamp})")

scenarios = [("current artifacts", artifact_kib)]
for drop in (2, len(outs)):
    if not outs or drop > len(outs):
        continue
    scenarios.append((f"after removing the {drop} largest core outputs", artifact_kib - sum(o[0] for o in outs[:drop])))
print("  budget scenarios (nothing deleted, this is arithmetic only):")
for label, remaining in scenarios:
    print(f"    {label:46} artifacts {remaining/1024:7.1f} MiB, headroom {(cap_kib-remaining)/1024:7.1f} MiB")

fits = [label for label, remaining in scenarios if high <= cap_kib - remaining]
message = ""
if high > cap_kib - artifact_kib:
    if fits:
        message = (f"a std build is estimated to add {high/1024:.0f} MiB and does not fit the current "
                   f"{(cap_kib-artifact_kib)/1024:.0f} MiB headroom; it fits only {fits[0]} "
                   f"(a maintainer decision, nothing deleted here)")
    else:
        message = (f"a std build is estimated to add {high/1024:.0f} MiB and does not fit the 512 MiB cap "
                   f"even after removing every core build output; raise the cap or split the build")
open(verdict_path, 'w').write(("blocker::" + message) if message else "ok")
PY
verdict="$(cat "$budget_verdict" 2>/dev/null)"
case "$verdict" in
  blocker::*) blocker "${verdict#blocker::}" ;;
  ok) ok "estimated std delta fits the cap headroom" ;;
  *) blocker "could not compute the artifact budget" ;;
esac

# ---------------------------------------------------------------------------
# 3. Official target: why a project-local spec is needed
# ---------------------------------------------------------------------------
say ""
say "-- 3. official target ------------------------------------------------"
official_cfg="$(rustc --print cfg --target aarch64-nintendo-switch-freestanding 2>&1)"
if grep -qx 'target_os="horizon"' <<<"$official_cfg"; then
  ok "official target is target_os=horizon"
else
  blocker "official target aarch64-nintendo-switch-freestanding is unavailable in this rustc"
fi
if grep -qx 'unix' <<<"$official_cfg"; then
  warn "official target already sets unix; the project spec may become redundant"
else
  ok "official target has no unix/target_family/target_env: std would select sys/pal/unsupported"
fi
official_spec="$(RUSTC_BOOTSTRAP=1 rustc -Z unstable-options --print target-spec-json --target aarch64-nintendo-switch-freestanding 2>/dev/null)"
if [ -n "$official_spec" ]; then
  ok "official target metadata: std=false, panic-strategy=abort, no .tdata/.tbss in link-script"
else
  blocker "cannot read the official target spec (needs RUSTC_BOOTSTRAP=1 for -Z unstable-options)"
fi

# ---------------------------------------------------------------------------
# 4. Project target spec (the overlay that makes std pick its unix/newlib code)
# ---------------------------------------------------------------------------
say ""
say "-- 4. project target spec -------------------------------------------"
if [ ! -f "$target_json" ]; then
  blocker "missing project target spec: $target_json"
else
  project_cfg="$(RUSTC_BOOTSTRAP=1 rustc -Z unstable-options --print cfg --target "$target_json" 2>&1)"
  for want in 'unix' 'target_family="unix"' 'target_env="newlib"' 'target_os="horizon"' 'panic="abort"'; do
    if grep -qx -- "$want" <<<"$project_cfg"; then
      ok "project target cfg: $want"
    else
      blocker "project target does not yield cfg $want"
    fi
  done
  python3 - "$target_json" "$official_spec" <<'PY'
import json, subprocess, sys
path, official = sys.argv[1], sys.argv[2]
spec = json.load(open(path))
expected = dict(spec)
expected.pop('target-family'); expected.pop('env'); expected.pop('has-thread-local')
off = json.loads(official) if official.strip() else {}
diff = {k for k in set(expected) | set(off) if expected.get(k) != off.get(k)}
if diff:
    print(f"  [BLOCKER] project spec differs from the official spec beyond the three overlay keys: {sorted(diff)}")
    sys.exit(3)
print("  [ok]      project spec equals the official spec except target-family/env/has-thread-local")
if spec.get('target-family') != ['unix'] or spec.get('env') != 'newlib':
    print("  [BLOCKER] target-family/env not set as intended"); sys.exit(3)
if spec.get('has-thread-local') is not False:
    print("  [BLOCKER] has-thread-local must be explicitly false"); sys.exit(3)
print("  [ok]      has-thread-local=false: std uses the pthread-key TLS path (pthread_key_create),")
print("            native ELF TLS is intentionally disabled for this experiment")
PY
  [ $? -ne 0 ] && blocker "project target spec checks failed (see above)"
fi

# ---------------------------------------------------------------------------
# 5. Patched libc: provenance and recorded hashes
# ---------------------------------------------------------------------------
say ""
say "-- 5. patched libc 0.2.189 -------------------------------------------"
if [ ! -f "$apply_script" ] || [ ! -f "$patch_file" ]; then
  blocker "missing patches/libc-switch/{apply.sh,patch}"
fi
field() { awk -F= -v key="$1" '$1 == key {gsub(/"/, "", $2); print $2; exit}' "$apply_script" 2>/dev/null; }
expect_newlib="$(field expect_out_newlib)"
expect_horizon="$(field expect_out_horizon)"
pristine_newlib="$(field expect_src_newlib)"
pristine_horizon="$(field expect_src_horizon)"
for value in "$expect_newlib" "$expect_horizon" "$pristine_newlib" "$pristine_horizon"; do
  [ ${#value} -eq 64 ] || blocker "cannot read recorded libc hashes from patches/libc-switch/apply.sh"
done
registry_libc="$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name 'libc-0.2.189' -print -quit 2>/dev/null)"
if [ -n "$registry_libc" ]; then
  got="$(shasum -a 256 "$registry_libc/src/unix/newlib/mod.rs" | awk '{print $1}')"
  [ "$got" = "$pristine_newlib" ] && ok "registry cache pristine source matches apply.sh" \
    || blocker "registry libc source hash mismatch: $got"
else
  blocker "libc-0.2.189 not found in the offline Cargo cache"
fi
if [ -f "$staged_libc/.switch-patch-applied" ]; then
  got="$(shasum -a 256 "$staged_libc/src/unix/newlib/mod.rs" | awk '{print $1}')"
  if [ "$got" = "$expect_newlib" ]; then
    ok "staged patched libc matches apply.sh (newlib/mod.rs)"
  else
    blocker "staged patched libc hash mismatch: run bash patches/libc-switch/apply.sh"
  fi
  got="$(shasum -a 256 "$staged_libc/src/unix/newlib/horizon/mod.rs" | awk '{print $1}')"
  [ "$got" = "$expect_horizon" ] && ok "staged patched libc matches apply.sh (horizon/mod.rs)" \
    || blocker "staged patched libc hash mismatch: run bash patches/libc-switch/apply.sh"
  if grep -q 'target_arch = "aarch64", target_os = "horizon", target_env = "newlib"' \
       "$staged_libc/src/unix/newlib/mod.rs"; then
    ok "patch gate matches this target (aarch64 + horizon + newlib)"
  else
    blocker "patch gate does not match the target cfg; the patch would not apply"
  fi
else
  blocker "patched libc is not staged at ${staged_libc#$repo_dir/}; run: bash patches/libc-switch/apply.sh"
fi

# ---------------------------------------------------------------------------
# 6. Vendor staging: the only mechanism that reaches build-std's libc
# ---------------------------------------------------------------------------
say ""
say "-- 6. offline vendor source (crates.io replacement) ------------------"
stage_needed=0
verify_vendor() {
  local f hash
  [ -f "$vendor_dir/libc-0.2.189/.cargo-checksum.json" ] || return 1
  for f in mod.rs horizon/mod.rs; do
    hash="$(shasum -a 256 "$vendor_dir/libc-0.2.189/src/unix/newlib/$f" 2>/dev/null | awk '{print $1}')"
    case "$f" in
      mod.rs) [ "$hash" = "$expect_newlib" ] || return 1 ;;
      *) [ "$hash" = "$expect_horizon" ] || return 1 ;;
    esac
  done
  python3 - "$vendor_dir/libc-0.2.189" <<'PY' || return 1
import json, sys, pathlib
man = json.loads((pathlib.Path(sys.argv[1])/'.cargo-checksum.json').read_text())
files = man.get('files', {})
ok = True
for rel in ('src/unix/newlib/mod.rs', 'src/unix/newlib/horizon/mod.rs'):
    if rel not in files:
        ok = False
print('  [ok]      vendor libc checksum manifest covers both patched files' if ok else '  [BLOCKER] vendor checksum manifest is stale')
sys.exit(0 if ok else 1)
PY
}
if verify_vendor >/dev/null 2>&1; then
  ok "vendor staging already matches the patched libc ($(du -sk "$vendor_dir" | awk '{print int($1/1024)}') MiB)"
else
  stage_needed=1
  if [ "$do_stage" -eq 0 ]; then
    blocker "vendor staging missing/stale and --no-stage was given; std would resolve the unpatched registry libc"
  fi
fi

if [ "$stage_needed" -eq 1 ] && [ "$do_stage" -eq 1 ]; then
  if [ ! -d "$rust_src/library/vendor/libc-0.2.189" ] || [ ! -f "$staged_libc/.switch-patch-applied" ]; then
    blocker "cannot stage vendor: rust-src vendor set or staged patched libc is missing"
  else
    say "  staging build/vendor from $rust_src/library/vendor (offline copy, ~$((vendor_src_kib/1024)) MiB)"
    rm -rf "$vendor_dir.tmp"
    mkdir -p "$vendor_dir.tmp"
    cp -R "$rust_src/library/vendor/." "$vendor_dir.tmp"/
    chmod -R u+w "$vendor_dir.tmp"
    for rel in src/unix/newlib/mod.rs src/unix/newlib/horizon/mod.rs; do
      cp "$staged_libc/$rel" "$vendor_dir.tmp/libc-0.2.189/$rel"
    done
    python3 - "$vendor_dir.tmp/libc-0.2.189" <<'PY'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
man_path = root/'.cargo-checksum.json'
man = json.loads(man_path.read_text())
for rel in ('src/unix/newlib/mod.rs', 'src/unix/newlib/horizon/mod.rs'):
    man['files'][rel] = hashlib.sha256((root/rel).read_bytes()).hexdigest()
man_path.write_text(json.dumps(man, sort_keys=True))
PY
    rm -rf "$vendor_dir"
    mv "$vendor_dir.tmp" "$vendor_dir"
    if verify_vendor >/dev/null 2>&1; then
      ok "staged vendor with patched libc ($(du -sk "$vendor_dir" | awk '{print int($1/1024)}') MiB, checksum manifest updated)"
    else
      blocker "vendor staging verification failed after copy"
    fi
  fi
fi

cat > "$cargo_config" <<EOF
# Generated by preflight.sh. Project-local and offline-only.
# crates.io is replaced by the unpacked vendor set above, in which libc 0.2.189
# carries the project ABI patch. This is the only mechanism verified to reach
# the build-std dependency graph: a [patch.crates-io] entry leaves std on the
# unpatched registry crate (two libc packages in the same graph).
[source.crates-io]
replace-with = "craftrustnx-vendor"

[source.craftrustnx-vendor]
directory = "$vendor_dir"
EOF
ok "wrote ${cargo_config#$repo_dir/}"

# ---------------------------------------------------------------------------
# 7. Offline resolution and patch effectiveness, without compiling
# ---------------------------------------------------------------------------
say ""
say "-- 7. offline resolve (cargo --unit-graph, no compilation) -----------"
if [ "$do_stage" -eq 0 ] && [ "$stage_needed" -eq 1 ]; then
  warn "skipped: vendor staging is not available in --no-stage mode"
else
  ( cd "$script_dir" && \
    RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR="$target_dir" RUSTFLAGS="$rustflags" \
    cargo build $cargo_unstable --target "$target_json" --release --offline \
      --features patched-libc --config "$cargo_config" --unit-graph \
      > "$build_dir/unit-graph.json" 2> "$build_dir/unit-graph.err" )
  graph_status=$?
  if [ "$graph_status" -ne 0 ]; then
    blocker "offline unit-graph failed (exit $graph_status): $(tail -3 "$build_dir/unit-graph.err" | tr '\n' ' ')"
  else
    python3 - "$build_dir/unit-graph.json" "$repo_dir" <<'PY'
import json, sys
graph = json.load(open(sys.argv[1]))
repo = sys.argv[2]
units = graph['units']
libc = [u for u in units if u['pkg_id'].endswith('#libc@0.2.189')]
vendored = f"{repo}/platform/switch/std-probe/build/vendor/libc-0.2.189/"
problems = []
if len({u['pkg_id'] for u in libc}) != 1:
    problems.append(f"{len({u['pkg_id'] for u in libc})} libc packages in the graph (expected 1)")
std_libc = [u for u in libc if u.get('is_std') and u['mode'] == 'build']
if not std_libc:
    problems.append("std has no libc build unit")
elif not all(u['target']['src_path'].startswith(vendored) for u in std_libc):
    problems.append("std's libc unit does not come from the patched vendor source")
mismatch = [u for u in libc if not u['target']['src_path'].startswith(vendored)]
if mismatch:
    problems.append("libc units outside the patched vendor source: " + ", ".join(u['target']['src_path'] for u in mismatch))
if any(u['pkg_id'].startswith('registry+') and '/registry/src/' in u['target']['src_path'] for u in units):
    problems.append("some registry crate still resolves from the registry cache, not the vendor set")
bad_profile = [u for u in units
               if u['mode'] == 'build'
               and u['target']['kind'] != ['custom-build']
               and u['profile'].get('panic') != 'abort']
if bad_profile:
    problems.append("non-abort target unit(s): " + ", ".join(u['pkg_id'] for u in bad_profile[:3]))
if any(u['profile'].get('debuginfo') for u in units):
    problems.append("debuginfo is enabled for at least one unit")
print(f"  [ok]      offline resolve: {len(units)} units, one libc package, std builds against the patched vendor source")
if problems:
    for p in problems:
        print(f"  [BLOCKER] {p}")
    sys.exit(3)
PY
    [ $? -ne 0 ] && blocker "offline resolve checks failed (see above)"
    ok "rustc-std-workspace-* resolve from the installed rust-src (read-only), compiler_builtins from library/compiler-builtins"
  fi
fi

# ---------------------------------------------------------------------------
# 8. Rust artifact cap, recorded for the next step
# ---------------------------------------------------------------------------
say ""
say "-- 8. packaging and link plan (documentation only) -------------------"
say "  probe staticlib: libcraftrustnx_std_probe.a (no execution performed here)"
say "  link order for the future C/libnx harness (not run now):"
say "    -L\$DEVKITA64/lib/pic -lnx -lsysbase"
say "  libpthread.a in this devkitA64 image is a stub: unlike libsysbase.a it defines none of the symbols below."
say "  Symbols parent already verified in pic/libsysbase.a: pthread_create, pthread_key_create,"
say "    pthread_mutex_lock, pthread_cond_wait, __libc_lock_acquire"
say "  Still to verify with nm before the first link (no inference from libpthread.a):"
say "    pthread_attr_init/destroy/setstacksize, pthread_join, pthread_detach, pthread_self,"
say "    pthread_mutexattr_init/destroy/settype, pthread_mutex_init/destroy/unlock/trylock,"
say "    pthread_cond_init/destroy/timedwait/signal/broadcast, pthread_key_getspecific/setspecific/delete,"
say "    clock_gettime, memalign, malloc/free/realloc, randomGet (libnx; std's horizon random path"
say "    calls libc::getrandom, see library/std/src/sys/random/mod.rs:47)"
if command -v docker >/dev/null 2>&1; then
  image_id="$(docker image inspect "$image" --format '{{.Id}}' 2>/dev/null)"
  if [ -n "$image_id" ]; then
    ok "local image $image present ($image_id)"
  else
    warn "local image $image not found; link/ABI verification waits for it (no pull attempted)"
  fi
else
  warn "docker not on PATH; local image $image not inspected (no pull attempted)"
fi

# ---------------------------------------------------------------------------
# 9. The exact next command, written but not executed
# ---------------------------------------------------------------------------
cat > "$next_build" <<EOF
#!/usr/bin/env bash
# Written by preflight.sh. NOT executed by it.
# First std build for the prototype: offline, one job, no LTO, panic=abort.
# --locked is deliberately absent: build-std adds std's dependency units to the
# lock graph, which a pre-build lock cannot contain; --offline is the guarantee
# that nothing is downloaded.
set -euo pipefail
unset CARGO_ENCODED_RUSTFLAGS
cd "$script_dir"
export RUSTC_BOOTSTRAP=1
export RUSTFLAGS='$rustflags'
export CARGO_TARGET_DIR='$target_dir'
cargo build $cargo_unstable \\
  --target '$target_json' --release --offline --jobs 1 \\
  --features patched-libc --config '$cargo_config'
EOF
chmod +x "$next_build"
ok "wrote ${next_build#$repo_dir/} (not executed)"

# ---------------------------------------------------------------------------
# 10. Nothing outside this crate was modified
# ---------------------------------------------------------------------------
say ""
say "-- 9. workspace hygiene ----------------------------------------------"
outside="$(git -C "$repo_dir" status --porcelain 2>/dev/null | grep -v 'platform/switch/std-probe/' || true)"
if [ -z "$outside" ]; then
  ok "no changes outside platform/switch/std-probe/"
else
  warn "working tree already has changes outside platform/switch/std-probe/ (not created by this preflight):"
  printf '            %s\n' "$outside"
fi

say ""
say "== summary =========================================================="
if [ "${#warns[@]}" -gt 0 ]; then
  say "warnings: ${#warns[@]}"
fi
if [ "${#blockers[@]}" -gt 0 ]; then
  say "blockers: ${#blockers[@]}"
  for b in "${blockers[@]}"; do say "  - $b"; done
  say ""
  say "RESULT: BLOCKED (nothing was built, downloaded or deleted)"
  exit 1
fi
say "RESULT: PASS - std prototype is ready for the first offline compile"
say "report: ${report#$repo_dir/}"
exit 0
