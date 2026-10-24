#!/usr/bin/env bash
# Nightfall PR gate: only sync non-conflicting, behind branches; never merge.
set -Eeuo pipefail

export GH_PROMPT_DISABLED=1
AUTO_UPDATE="${AUTO_UPDATE:-1}"
REPORT="${REPORT:-$HOME/nightfall-pr-gate-$(date -u +%Y%m%dT%H%M%SZ).md}"
REPOS=("Instinctes/nightfall" "nightfall-wizard/nightfall" "nightfall-wizard/nightfall-wizard-fork")

for cmd in gh jq git; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Fehlt: $cmd. Einmalig installieren mit: pkg install gh jq git" >&2
    exit 2
  fi
done
if ! gh auth status -h github.com >/dev/null 2>&1; then
  echo "GitHub-Anmeldung nötig (kein Token im Skript)." >&2
  unset GH_PROMPT_DISABLED
  gh auth login -h github.com -p https -w
  export GH_PROMPT_DISABLED=1
fi
ME="$(gh api user --jq .login)"
if [[ "$ME" != "nightfall-wizard" ]]; then
  echo "Sicherheitsstopp: angemeldet als '$ME' statt nightfall-wizard." >&2
  exit 3
fi

printf '# Nightfall PR Release Gate\n\nUTC: %s\nKonto: %s\n\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$ME" > "$REPORT"
printf '%s\n' 'Keine automatischen Merges, Approvals, Force-Pushes oder Konflikt-Auflösungen.' >> "$REPORT"
printf '%s\n\n' 'Ein grüner Gate-Status ersetzt keinen Security-Audit und keine Release-Freigabe.' >> "$REPORT"
printf '%s\n' '| Repo / PR | Status | CI | Review | Merge | Risiko |' '|---|---|---|---|---|---|' >> "$REPORT"

TOTAL=0; READY=0; UPDATED=0; BLOCKED=0; ERRORS=0
for REPO in "${REPOS[@]}"; do
  echo "Prüfe $REPO ..."
  if ! LIST="$(gh pr list -R "$REPO" --state open --author "$ME" --limit 100 \
    --json number,title,url,isDraft,mergeable,mergeStateStatus,reviewDecision,headRepositoryOwner)"; then
    echo "Fehler: PR-Liste für $REPO nicht lesbar" >&2
    ERRORS=$((ERRORS+1))
    continue
  fi
  while IFS= read -r ITEM; do
    [[ -n "$ITEM" ]] || continue
    TOTAL=$((TOTAL+1))
    NUM="$(jq -r '.number' <<< "$ITEM")"
    URL="$(jq -r '.url' <<< "$ITEM")"
    TITLE="$(jq -r '.title | gsub("[|\\r\\n]"; " ")' <<< "$ITEM")"
    DRAFT="$(jq -r '.isDraft' <<< "$ITEM")"
    MERGEABLE="$(jq -r '.mergeable // "UNKNOWN"' <<< "$ITEM")"
    MERGE_STATE="$(jq -r '.mergeStateStatus // "UNKNOWN"' <<< "$ITEM")"
    REVIEW="$(jq -r '.reviewDecision // "REVIEW_REQUIRED"' <<< "$ITEM")"
    HEAD_OWNER="$(jq -r '.headRepositoryOwner.login // ""' <<< "$ITEM")"

    # gh pr checks can return a nonzero exit status for failed checks;
    # retain its JSON stdout rather than classifying that as a query failure.
    CHECKS="$(gh pr checks "$NUM" -R "$REPO" --json bucket 2>/dev/null || true)"
    CI="UNKNOWN"
    if jq -e 'type == "array"' >/dev/null 2>&1 <<< "$CHECKS"; then
      if jq -e 'length == 0' >/dev/null <<< "$CHECKS"; then
        CI="NO_CHECKS"
      elif jq -e 'any(.[]; .bucket == "fail" or .bucket == "cancel")' >/dev/null <<< "$CHECKS"; then
        CI="FAILED"
      elif jq -e 'any(.[]; .bucket == "pending")' >/dev/null <<< "$CHECKS"; then
        CI="PENDING"
      elif jq -e 'all(.[]; .bucket == "pass" or .bucket == "skipping") and any(.[]; .bucket == "pass")' >/dev/null <<< "$CHECKS"; then
        CI="PASSED"
      fi
    fi

    # Only GitHub's conflict-free update; do not invent conflict resolutions.
    if [[ "$AUTO_UPDATE" == 1 && "$DRAFT" == false && "$HEAD_OWNER" == "$ME" \
       && "$MERGEABLE" == MERGEABLE && "$MERGE_STATE" == BEHIND ]]; then
      if gh pr update-branch "$NUM" -R "$REPO" >/dev/null 2>&1; then
        UPDATED=$((UPDATED+1))
        CI="PENDING"
        MERGE_STATE="UPDATED"
        echo "  #$NUM: Branch synchronisiert; CI muss neu durchlaufen"
      else
        echo "  #$NUM: Update von GitHub abgelehnt" >&2
        ERRORS=$((ERRORS+1))
      fi
    fi

    # Files identify heightened audit attention, not an automated veto.
    RISK="STANDARD"
    FILES="$(gh pr view "$NUM" -R "$REPO" --json files --jq '.files[].path' 2>/dev/null || true)"
    if grep -Eq '^(crates/nightfall-(crypto|consensus|ledger|types|wallet|storage|p2p)/|docs/SPEC\.md|Cargo\.lock$)' <<< "$FILES"; then
      RISK="SECURITY_REVIEW"
    fi

    GATE="BLOCKED"
    if [[ "$DRAFT" == false && "$MERGEABLE" == MERGEABLE && "$CI" == PASSED && "$REVIEW" == APPROVED ]]; then
      GATE="GATES_GREEN"
      READY=$((READY+1))
    else
      BLOCKED=$((BLOCKED+1))
    fi
    if [[ "$DRAFT" == true ]]; then GATE="DRAFT"; fi
    if [[ "$MERGEABLE" == CONFLICTING ]]; then GATE="CONFLICT"; fi
    printf '| [%s #%s](%s) %s | %s | %s | %s | %s | %s |\n' \
      "$REPO" "$NUM" "$URL" "$TITLE" "$GATE" "$CI" "$REVIEW" "$MERGE_STATE" "$RISK" >> "$REPORT"
    echo "  #$NUM: $GATE / CI=$CI / review=$REVIEW / merge=$MERGE_STATE"
  done < <(jq -c '.[]' <<< "$LIST")
done

{
  printf '\n## Gesamtstand\n\n'
  printf -- '- Offen geprüft: %s\n- Gates grün: %s\n- Blockiert/Entwürfe: %s\n' "$TOTAL" "$READY" "$BLOCKED"
  printf -- '- Konfliktfrei aktualisierte Branches: %s\n- API-/Update-Fehler: %s\n' "$UPDATED" "$ERRORS"
  printf '\n## Vor Release zusätzlich verpflichtend\n\n'
  printf '%s\n' 'Maintainer-Review, sämtliche Plattform-Builds und Regressionstests, unabhängige Sicherheitsprüfung bei kritischen Änderungen, Versions-/Protokoll-Kompatibilität und Freigabe der Artefakte.'
} >> "$REPORT"
cat "$REPORT"
echo "Bericht gespeichert: $REPORT"
if (( BLOCKED > 0 || ERRORS > 0 )); then exit 1; fi
