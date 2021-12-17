#!/bin/bash
# Regenerate docs/cmd/ from long help text

set -euo pipefail

rootdir="$(dirname $0)/.."
prog="${rootdir}/target/${PROFILE:-debug}/coreos-installer"

generate() {
    local subcommand subcommands
    subcommands=$("${prog}" $* -h | awk 'BEGIN {subcommands=0} {if (subcommands) print $1} /SUBCOMMANDS:/ {subcommands=1}')

    # Generate TOC if this is a root command with subcommands
    if [ -n "${subcommands}" -a $# = 1 ]; then
        cat <<EOF

# coreos-installer $*
{: .no_toc }

1. TOC
{:toc}
EOF
    fi

    # Generate docs if this subcommand doesn't have subcommands
    if [ -z "${subcommands}" ]; then
        cat <<EOF

# coreos-installer $*

\`\`\`
EOF
        # Run coreos-installer in a PTY and override the terminal width to
        # the one we want, regardless of the caller's terminal.  More than
        # 100 characters will cause the code block to scroll.
        # Drop CR characters added by `script`.
        # Drop first line with incorrectly hyphenated command name and version
        # Fix trailing whitespace
        script -qc "stty cols 95 rows 24; ${prog} $* --help" /dev/null |
            tr -d '\r' |
            tail -n +2 |
            sed 's/[[:blank:]]*$//'
        echo '```'
    fi

    # Recurse
    for subcommand in ${subcommands}; do
        generate $* ${subcommand}
    done
}

for path in ${rootdir}/docs/cmd/*.md; do
    echo "Generating $(realpath --relative-to=${rootdir} ${path})..."
    # copy YAML metadata
    awk '/^$/ {exit} {print}' "${path}" > "${path}.new"
    generate "$(basename ${path} .md)" >> "${path}.new"
    mv "${path}.new" "${path}"
done
