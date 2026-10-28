#!/usr/bin/env bash
set -euo pipefail
: "${AUTOMONIQUE_DEPENDENCY_KEY:?The trusted build requires AUTOMONIQUE_READ_DEPLOY_KEY}"
: "${RUNNER_TEMP:?}"
: "${GITHUB_ENV:?}"
: "${GITHUB_ACTION_PATH:?}"
umask 077
runner_temp="$RUNNER_TEMP"
if command -v cygpath >/dev/null 2>&1; then
    runner_temp="$(cygpath -u "$runner_temp")"
fi
key_path="$(mktemp "$runner_temp/shelldeck-automonique-key.XXXXXX")"
hosts_path="$(mktemp "$runner_temp/shelldeck-automonique-hosts.XXXXXX")"
configured=no
trap 'if [[ "$configured" != yes ]]; then rm -f -- "$key_path" "$hosts_path"; fi' EXIT
printf '%s\n' "$AUTOMONIQUE_DEPENDENCY_KEY" > "$key_path"
unset AUTOMONIQUE_DEPENDENCY_KEY
chmod 600 "$key_path"
if [[ "${RUNNER_OS:-}" == Windows ]]; then
    # Git Bash chmod does not narrow native Windows ACL inheritance.
    MSYS2_ARG_CONV_EXCL='*' icacls "$(cygpath -w "$key_path")" /inheritance:r /grant:r "${USERNAME}:R" >/dev/null
fi
cp "$GITHUB_ACTION_PATH/github_known_hosts" "$hosts_path"
ssh_binary="$(command -v ssh)"
printf -v ssh_command '%q -F none -i %q -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=%q' "$ssh_binary" "$key_path" "$hosts_path"
# This rewrite cannot redirect another dependency or change checkout credentials.
git config --global url.ssh://git@github.com/bext-stack/automonique.insteadOf https://github.com/bext-stack/automonique
GIT_SSH_COMMAND="$ssh_command" GIT_TERMINAL_PROMPT=0 git ls-remote ssh://git@github.com/bext-stack/automonique HEAD >/dev/null
{
    printf 'CARGO_NET_GIT_FETCH_WITH_CLI=true\n'
    printf 'GIT_SSH_COMMAND=%s\n' "$ssh_command"
    printf 'AUTOMONIQUE_DEPENDENCY_KEY_PATH=%s\n' "$key_path"
    printf 'AUTOMONIQUE_DEPENDENCY_HOSTS_PATH=%s\n' "$hosts_path"
} >> "$GITHUB_ENV"
configured=yes
