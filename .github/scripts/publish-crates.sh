#!/usr/bin/env bash
#
# Publish every publishable workspace crate to crates.io, retrying when
# crates.io rate-limits us.
#
# Why this exists: the workspace has ~33 publishable crates and crates.io
# throttles publishes (as of writing: new versions of an existing crate are a
# burst of 30 then 1 per minute; brand-new crates are a burst of 5 then 1 per
# 10 minutes).  A full workspace publish therefore reliably ends in
#
#     the remote server responded with an error (status 429 Too Many Requests):
#     You have published too many crates ... Please try again after <timestamp>
#
# after the burst budget is spent, leaving the tail of the workspace
# unpublished.  Publishing is idempotent, so the fix is to wait for the bucket
# to refill and re-run for whatever is still missing.
#
# Each attempt recomputes which crates are already live at the target version
# and excludes them, so no crate is ever published twice and a partial success
# always makes forward progress.
#
# Tunables (environment):
#   PUBLISH_MAX_ATTEMPTS  attempts before giving up          (default 5)
#   PUBLISH_BASE_SLEEP    seconds to wait after a rate limit (default 300)
#   PUBLISH_MAX_SLEEP     cap on any single wait             (default 1800)
#   PUBLISH_ERROR_SLEEP   wait after a non-rate-limit error  (default 60)
#   PUBLISH_TIMEOUT       per-crate registry confirmation    (default 600)

set -euo pipefail

MAX_ATTEMPTS="${PUBLISH_MAX_ATTEMPTS:-5}"
BASE_SLEEP="${PUBLISH_BASE_SLEEP:-300}"
MAX_SLEEP="${PUBLISH_MAX_SLEEP:-1800}"
ERROR_SLEEP="${PUBLISH_ERROR_SLEEP:-60}"
PUBLISH_TIMEOUT="${PUBLISH_TIMEOUT:-600}"

USER_AGENT="clojurust-publish/1.0"

log() { printf '%s\n' "$*"; }

# Is <name>@<version> already on crates.io?
crate_is_published() {
  local name="$1" version="$2"
  curl -sf "https://crates.io/api/v1/crates/${name}/${version}" \
    -H "User-Agent: ${USER_AGENT}" | grep -q '"num"'
}

# Every publishable workspace package as "<name> <version>" lines.
# cargo sets `publish` to [] for `publish = false` packages and null otherwise.
workspace_packages() {
  cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select(.publish != []) | "\(.name) \(.version)"'
}

# Fill the globals REMAINING (names still to publish) and EXCLUDE_ARGS
# (--exclude flags for the ones already live).
compute_remaining() {
  REMAINING=()
  EXCLUDE_ARGS=()
  local name version
  while read -r name version; do
    if crate_is_published "$name" "$version"; then
      EXCLUDE_ARGS+=(--exclude "$name")
    else
      REMAINING+=("${name}@${version}")
    fi
  done < <(workspace_packages)
}

# Does the cargo output look like a crates.io rate limit rather than a real
# failure (compile error, bad token, ...)?
is_rate_limited() {
  grep -qiE '429|too many requests|published too many' "$1"
}

# Seconds to wait, preferring the timestamp crates.io hands back:
#   "Please try again after 2026-08-12T18:12:00Z or email help@crates.io ..."
# Falls back to attempt-scaled backoff when no timestamp can be parsed.
rate_limit_sleep() {
  local log_file="$1" attempt="$2"
  local stamp delta fallback

  fallback=$((BASE_SLEEP * attempt))

  stamp="$(grep -oiE 'try again after [0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:?[0-9]{2})?' "$log_file" \
    | tail -n 1 | sed -E 's/^[Tt]ry again after //')"

  if [[ -n "$stamp" ]] && delta="$(date -u -d "$stamp" +%s 2>/dev/null)"; then
    # +30 s of slack for clock skew between the runner and crates.io.
    delta=$((delta - $(date -u +%s) + 30))
    if ((delta > 0)); then
      clamp "$delta"
      return
    fi
  fi

  clamp "$fallback"
}

# Echo $1 bounded to [60, MAX_SLEEP].
clamp() {
  local seconds="$1"
  ((seconds < 60)) && seconds=60
  ((seconds > MAX_SLEEP)) && seconds="$MAX_SLEEP"
  printf '%s\n' "$seconds"
}

# `--no-verify` is safe here: the CI job that gates this one already built,
# clippy'd and tested the workspace.
# Nightly + `-Zpublish-timeout` extends the per-crate registry confirmation
# wait; without it a slow crates.io response aborts the run with
# "no packages ready to publish ... 1 awaiting confirmation"
# (rust-lang/cargo#17028).
run_cargo_publish() {
  local log_file="$1"
  cargo +nightly publish --workspace \
    -Zpublish-timeout \
    --config "publish.timeout=${PUBLISH_TIMEOUT}" \
    --no-verify \
    "${EXCLUDE_ARGS[@]}" 2>&1 | tee "$log_file"
  # tee is last in the pipeline, so consult cargo's status explicitly.
  return "${PIPESTATUS[0]}"
}

log_file="$(mktemp)"
trap 'rm -f "$log_file"' EXIT

for ((attempt = 1; attempt <= MAX_ATTEMPTS; attempt++)); do
  compute_remaining

  if ((${#REMAINING[@]} == 0)); then
    log "All workspace crates are published at their target versions."
    exit 0
  fi

  if ((${#EXCLUDE_ARGS[@]} > 0)); then
    log "Already published, skipping: $((${#EXCLUDE_ARGS[@]} / 2)) crate(s)"
  fi
  log "Attempt ${attempt}/${MAX_ATTEMPTS}: publishing ${#REMAINING[@]} crate(s): ${REMAINING[*]}"

  if run_cargo_publish "$log_file"; then
    # The loop head re-checks the registry; give crates.io a moment to make the
    # just-published versions visible so that check isn't racing the API.
    log "cargo publish reported success; confirming against the registry."
    sleep 30
    continue
  fi

  if ((attempt == MAX_ATTEMPTS)); then
    break
  fi

  if is_rate_limited "$log_file"; then
    sleep_for="$(rate_limit_sleep "$log_file" "$attempt")"
    log "::warning::crates.io rate-limited the publish; waiting ${sleep_for}s before retrying"
  else
    sleep_for="$(clamp "$ERROR_SLEEP")"
    log "::warning::cargo publish failed for a non-rate-limit reason; waiting ${sleep_for}s before retrying"
  fi
  sleep "$sleep_for"
done

# cargo can exit non-zero on the last crate while everything actually landed,
# so the registry — not cargo's exit status — has the final word.
compute_remaining
if ((${#REMAINING[@]} == 0)); then
  log "All workspace crates are published at their target versions."
  exit 0
fi

log "::error::Gave up after ${MAX_ATTEMPTS} attempt(s). Still unpublished: ${REMAINING[*]}"
log "Publishing is idempotent — re-running this workflow will publish only the crates listed above."
exit 1
