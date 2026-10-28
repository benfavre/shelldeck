#!/usr/bin/env bash
set -euo pipefail
runner_temp="${RUNNER_TEMP:?}"
if command -v cygpath >/dev/null 2>&1; then
    runner_temp="$(cygpath -u "$runner_temp")"
fi
for file in "${AUTOMONIQUE_DEPENDENCY_KEY_PATH:-}" "${AUTOMONIQUE_DEPENDENCY_HOSTS_PATH:-}"; do
    case "$file" in
        "$runner_temp"/shelldeck-automonique-*)
            relative="${file#"$runner_temp"/}"
            if [[ "$relative" == */* ]]; then
                printf 'Refusing a nested dependency credential path\n' >&2
                exit 1
            fi
            rm -f -- "$file"
            ;;
        '') ;;
        *) printf 'Refusing an unexpected dependency credential path\n' >&2; exit 1 ;;
    esac
done
