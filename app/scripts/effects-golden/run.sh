#!/bin/sh
# Regenerates crates/modlight-effects/tests/golden from upstream's own effect classes.
# Runs ONLY inside a throwaway Docker container (mcr.microsoft.com/dotnet/sdk:10.0,
# SkiaSharp 3.119.4 with its Linux native assets); nothing from upstream runs on the host.
#
#   scripts/effects-golden/run.sh [comma-separated effect ids]
#
# UPSTREAM: the upstream checkout (default ../upstream/Modlight-App next to the repo).
# OUT: where the harness writes (default crates/modlight-effects/tests/golden). A run
# limited to some ids writes only their cases.json entries: merge them by name.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
APP=$(cd "$HERE/../.." && pwd)
UPSTREAM=${UPSTREAM:-$(cd "$APP/../.." && pwd)/upstream/Modlight-App}
OUT=${OUT:-$APP/crates/modlight-effects/tests/golden}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/effects-golden.XXXXXX")
cp "$HERE/Program.cs" "$HERE/Harness.csproj" "$WORK/"
mkdir -p "$OUT" "$WORK/nuget"
docker run --rm --name modlight-fx-golden \
  -v "$UPSTREAM":/upstream:ro \
  -v "$WORK":/harness \
  -v "$OUT":/out \
  -v "$WORK/nuget":/root/.nuget/packages \
  -w /harness mcr.microsoft.com/dotnet/sdk:10.0 \
  sh -c "dotnet run -c Release -- /out '${1:-}'"
rm -rf "$WORK"
