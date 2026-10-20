#!/bin/sh
# Entrypoint of the Modlight (Rust) image: size the runtime to the cgroup, then
# run `modlight-server serve`, which validates the configuration, applies the
# database migrations (what upstream's `node dist/migrate.js` did) and serves.
# Any other argument is passed through (`docker run <image> migrate`).
set -e

# ── Size the runtime to the CGROUP, not to the host ──────────────────────────
#
# Same reasoning as upstream's docker/entrypoint.sh: inside the container the
# machine the process can see is the whole VPS. Upstream sized the V8 heap and
# libuv's pool here, and libvips' threads from JavaScript (sharp-runtime.ts).
# A Rust process has no heap ceiling to set; what it does have is libvips'
# thread count and the async runtime's worker pool, both sized to the CPU quota
# (`cpus:` in compose) instead of the host's core count. The binary computes
# the same number itself (src/runtime/limits.rs, src/media/sharp_runtime.rs:
# sharp.concurrency(effectiveCpuCount())); the variables make libvips' own
# default agree from the first instruction, and an explicit setting wins.

cgroup_bytes() {
  for f in /sys/fs/cgroup/memory.max /sys/fs/cgroup/memory/memory.limit_in_bytes; do
    if [ -r "$f" ]; then
      v=$(cat "$f")
      case "$v" in
        max|"") ;;
        *) [ "$v" -gt 0 ] 2>/dev/null && echo "$v" && return ;;
      esac
    fi
  done
}

# Whole cores of the CPU quota (cpu.max "<quota> <period>"), rounded down,
# never below 1; nothing when the cgroup sets no quota.
cgroup_cpus() {
  if [ -r /sys/fs/cgroup/cpu.max ]; then
    read -r quota period < /sys/fs/cgroup/cpu.max || return 0
    case "$quota" in
      max|"") return 0 ;;
    esac
    [ -n "$period" ] && [ "$period" -gt 0 ] 2>/dev/null || period=100000
    n=$((quota / period))
    [ "$n" -lt 1 ] && n=1
    echo "$n"
  fi
}

MEM_BYTES=$(cgroup_bytes || true)
if [ -n "$MEM_BYTES" ] && [ "$MEM_BYTES" -lt 68719476736 ]; then
  echo "→ memory limit ${MEM_BYTES} bytes"
fi

CPUS=$(cgroup_cpus || true)
if [ -n "$CPUS" ]; then
  export VIPS_CONCURRENCY="${VIPS_CONCURRENCY:-$CPUS}"
  export TOKIO_WORKER_THREADS="${TOKIO_WORKER_THREADS:-$CPUS}"
  echo "→ cpu limit ${CPUS} core(s); VIPS_CONCURRENCY=${VIPS_CONCURRENCY} TOKIO_WORKER_THREADS=${TOKIO_WORKER_THREADS}"
fi

if [ "$#" -eq 0 ]; then
  set -- serve
fi
if [ "$1" = serve ]; then
  echo "→ Applying database migrations, then starting Modlight…"
fi
exec /usr/local/bin/modlight-server "$@"
