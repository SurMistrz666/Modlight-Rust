#!/bin/sh
# Regenerates crates/modlight-effects/tests/golden from upstream's own effect classes.
# Runs ONLY inside a throwaway Docker container (mcr.microsoft.com/dotnet/sdk:10.0,
# SkiaSharp 3.119.4 with its Linux native assets); nothing from upstream runs on the host.
#
#   scripts/effects-golden/run.sh [comma-separated effect ids]
#   EDGE=1 scripts/effects-golden/run.sh     # the cases of edge.txt into tests/golden-edge
#
# UPSTREAM: the upstream checkout (default upstream/Modlight-App of the main checkout,
# also when run from a worktree). OUT: where the harness writes (default
# crates/modlight-effects/tests/golden, or tests/golden-edge with EDGE=1). A run limited
# to some ids writes only their cases.json entries: merge them by name. The work folder
# is made under target/: the Docker VM (colima) shares the home folder, not the
# system's temporary one.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
APP=$(cd "$HERE/../.." && pwd)
ROOT=$(cd "$(git -C "$APP" rev-parse --path-format=absolute --git-common-dir)/.." && pwd)
UPSTREAM=${UPSTREAM:-$ROOT/upstream/Modlight-App}
if [ "${EDGE:-}" = 1 ]; then
  OUT=${OUT:-$APP/crates/modlight-effects/tests/golden-edge}
  REQUEST=/harness/edge.txt
else
  OUT=${OUT:-$APP/crates/modlight-effects/tests/golden}
  REQUEST=
fi
mkdir -p "$APP/target"
WORK=$(mktemp -d "$APP/target/effects-golden.XXXXXX")
cp "$HERE/Program.cs" "$HERE/Harness.csproj" "$HERE/edge.txt" "$WORK/"
mkdir -p "$OUT" "$WORK/nuget"
docker run --rm --name modlight-fx-golden \
  -v "$UPSTREAM":/upstream:ro \
  -v "$WORK":/harness \
  -v "$OUT":/out \
  -v "$WORK/nuget":/root/.nuget/packages \
  -w /harness mcr.microsoft.com/dotnet/sdk:10.0 \
  sh -c "dotnet run -c Release -- /out '${1:-}' '$REQUEST'"
rm -rf "$WORK"
